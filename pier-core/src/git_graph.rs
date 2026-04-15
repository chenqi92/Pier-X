//! Git Graph — direct `.git` access via libgit2 for high-performance graph data.
//!
//! Replaces process-spawned `git log`, `git branch`, etc. with in-process reads.
//! Also provides IDEA-style graph layout computation (lane assignment, segments, arrows).
//!
//! Ported from the sibling `pier` project. Documentation warnings
//! are suppressed — the public API is documented at the FFI layer.
#![allow(missing_docs)]

use git2::{BranchType, Repository, Sort};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::process_util::configure_background_command;

// ═══════════════════════════════════════════════════════════
// Data types
// ═══════════════════════════════════════════════════════════

/// Filter options for graph log queries.
pub struct GraphFilter {
    pub branch: Option<String>,
    pub author: Option<String>,
    pub search_text: Option<String>,
    pub after_timestamp: i64, // 0 = no filter
    pub topo_order: bool,
    pub first_parent_only: bool,
    pub no_merges: bool,
    pub paths: Vec<String>,
}

#[derive(Serialize)]
pub struct CommitEntry {
    pub hash: String,
    pub parents: String,
    pub short_hash: String,
    pub refs: String,
    pub message: String,
    pub author: String,
    pub date_timestamp: i64,
}

// ═══════════════════════════════════════════════════════════
// Helper: build ref decoration string for a commit
// ═══════════════════════════════════════════════════════════

fn build_ref_decoration(repo: &Repository, commit_id: git2::Oid) -> String {
    let mut decorations = Vec::new();

    // Check HEAD
    if let Ok(head) = repo.head() {
        if let Some(target) = head.target() {
            if target == commit_id {
                if head.is_branch() {
                    if let Some(name) = head.shorthand() {
                        decorations.push(format!("HEAD -> {name}"));
                    } else {
                        decorations.push("HEAD".to_string());
                    }
                } else {
                    decorations.push("HEAD".to_string());
                }
            }
        }
    }

    // Check branches
    if let Ok(branches) = repo.branches(None) {
        for (branch, _btype) in branches.flatten() {
            if let Ok(Some(reference)) = branch.get().resolve().map(|r| r.target()) {
                if reference == commit_id {
                    if let Ok(Some(name)) = branch.name() {
                        // Skip if already added as HEAD ->
                        if !decorations.iter().any(|d| d.contains(name)) {
                            decorations.push(name.to_string());
                        }
                    }
                }
            }
        }
    }

    // Check tags
    if let Ok(tags) = repo.tag_names(None) {
        for tag_name in tags.iter().flatten() {
            if let Ok(reference) = repo.find_reference(&format!("refs/tags/{tag_name}")) {
                let target = if let Ok(tag) = reference.peel_to_commit() {
                    tag.id()
                } else if let Some(t) = reference.target() {
                    t
                } else {
                    continue;
                };
                if target == commit_id {
                    decorations.push(format!("tag: {tag_name}"));
                }
            }
        }
    }

    if decorations.is_empty() {
        String::new()
    } else {
        format!(" ({})", decorations.join(", "))
    }
}

// ═══════════════════════════════════════════════════════════
// Core functions
// ═══════════════════════════════════════════════════════════

/// Load commit graph data with filters. Returns a list of CommitEntry.
///
/// Uses `git log --topo-order --date-order` subprocess to ensure commit
/// ordering matches IntelliJ IDEA exactly. Ref decorations are enriched
/// via libgit2.
pub fn graph_log(
    repo_path: &str,
    limit: usize,
    skip: usize,
    filter: &GraphFilter,
) -> Result<Vec<CommitEntry>, String> {
    use std::process::Command;

    // Open repo with libgit2 only for ref decoration
    let repo = Repository::open(repo_path).map_err(|e| format!("Failed to open repo: {e}"))?;

    // Build git log command
    // Format: hash<SEP>parents<SEP>message<SEP>author<SEP>timestamp
    let separator = "\x1f"; // ASCII Unit Separator
    let format_str = format!("%H{separator}%P{separator}%s{separator}%an{separator}%ct");

    let mut cmd = Command::new("git");
    cmd.current_dir(repo_path);
    cmd.arg("log");
    if filter.topo_order {
        cmd.arg("--topo-order");
    } else {
        cmd.arg("--date-order");
    }
    cmd.args([&format!("--format={format_str}")]);
    configure_background_command(&mut cmd);

    // Limit & skip
    cmd.arg(format!("-n{}", limit + skip)); // fetch enough to skip + limit
                                            // We handle skip ourselves after parsing to support path-filter skipping

    // First-parent only
    if filter.first_parent_only {
        cmd.arg("--first-parent");
    }

    // No merges
    if filter.no_merges {
        cmd.arg("--no-merges");
    }

    // Author filter
    if let Some(ref author) = filter.author {
        cmd.arg(format!("--author={author}"));
    }

    // Search text (grep commit message)
    if let Some(ref search) = filter.search_text {
        cmd.arg(format!("--grep={search}"));
        cmd.arg("-i"); // case insensitive
    }

    // After date
    if filter.after_timestamp > 0 {
        cmd.arg(format!("--after={}", filter.after_timestamp));
    }

    // Branch filter or all refs
    if let Some(ref branch_name) = filter.branch {
        cmd.arg(branch_name.as_str());
    } else {
        cmd.arg("--branches").arg("--remotes").arg("--tags");
    }

    // Path filter (must come after --)
    if !filter.paths.is_empty() {
        cmd.arg("--");
        for path in &filter.paths {
            cmd.arg(path.as_str());
        }
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run git log: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git log failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut results = Vec::with_capacity(limit);
    let mut skipped = 0;

    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.splitn(5, '\x1f').collect();
        if parts.len() < 5 {
            continue;
        }

        let hash = parts[0].to_string();
        let parents = parts[1].to_string();
        let message = parts[2].to_string();
        let author = parts[3].to_string();
        let date_timestamp: i64 = parts[4].parse().unwrap_or(0);

        // Skip N commits
        if skipped < skip {
            skipped += 1;
            continue;
        }

        let short_hash = hash[..8.min(hash.len())].to_string();
        let commit_oid = git2::Oid::from_str(&hash).ok();
        let refs_str = commit_oid
            .map(|oid| build_ref_decoration(&repo, oid))
            .unwrap_or_default();

        results.push(CommitEntry {
            hash,
            parents,
            short_hash,
            refs: refs_str,
            message,
            author,
            date_timestamp,
        });

        if results.len() >= limit {
            break;
        }
    }

    Ok(results)
}

/// Check if a commit touches any of the specified paths.
#[allow(dead_code)]
fn commit_touches_paths(repo: &Repository, commit: &git2::Commit<'_>, paths: &[String]) -> bool {
    let tree = match commit.tree() {
        Ok(t) => t,
        Err(_) => return false,
    };

    if commit.parent_count() == 0 {
        // Root commit: check if any path exists in the tree
        for path in paths {
            if tree.get_path(Path::new(path)).is_ok() {
                return true;
            }
        }
        return false;
    }

    // Compare with first parent
    let parent_tree = match commit.parent(0).and_then(|p| p.tree()) {
        Ok(t) => t,
        Err(_) => return false,
    };

    let diff = match repo.diff_tree_to_tree(Some(&parent_tree), Some(&tree), None) {
        Ok(d) => d,
        Err(_) => return false,
    };

    for delta in diff.deltas() {
        let old_path = delta
            .old_file()
            .path()
            .and_then(|p| p.to_str())
            .unwrap_or("");
        let new_path = delta
            .new_file()
            .path()
            .and_then(|p| p.to_str())
            .unwrap_or("");
        for filter_path in paths {
            if old_path.starts_with(filter_path.as_str())
                || new_path.starts_with(filter_path.as_str())
            {
                return true;
            }
        }
    }
    false
}

/// Get the first-parent chain hashes for a given ref.
pub fn first_parent_chain(
    repo_path: &str,
    ref_name: &str,
    limit: usize,
) -> Result<Vec<String>, String> {
    let repo = Repository::open(repo_path).map_err(|e| format!("Failed to open repo: {e}"))?;

    let mut revwalk = repo
        .revwalk()
        .map_err(|e| format!("Failed to create revwalk: {e}"))?;
    revwalk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME).ok();
    revwalk.simplify_first_parent().ok();

    // Push the ref
    if let Ok(reference) = repo
        .find_reference(&format!("refs/heads/{ref_name}"))
        .or_else(|_| repo.find_reference(&format!("refs/remotes/{ref_name}")))
        .or_else(|_| repo.find_reference(ref_name))
    {
        if let Some(target) = reference.target() {
            revwalk.push(target).ok();
        } else if let Ok(resolved) = reference.resolve() {
            if let Some(target) = resolved.target() {
                revwalk.push(target).ok();
            }
        }
    } else if ref_name == "HEAD" {
        revwalk.push_head().ok();
    } else if let Ok(oid) = git2::Oid::from_str(ref_name) {
        revwalk.push(oid).ok();
    }

    let mut hashes = Vec::with_capacity(limit);
    for oid in revwalk.flatten() {
        hashes.push(oid.to_string());
        if hashes.len() >= limit {
            break;
        }
    }
    Ok(hashes)
}

/// List all branch names (local + remote).
pub fn list_branches(repo_path: &str) -> Result<Vec<String>, String> {
    let repo = Repository::open(repo_path).map_err(|e| format!("Failed to open repo: {e}"))?;

    let mut names = Vec::new();
    if let Ok(branches) = repo.branches(Some(BranchType::Local)) {
        for b in branches.flatten() {
            if let Ok(Some(name)) = b.0.name() {
                names.push(name.to_string());
            }
        }
    }
    if let Ok(branches) = repo.branches(Some(BranchType::Remote)) {
        for b in branches.flatten() {
            if let Ok(Some(name)) = b.0.name() {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

/// List unique commit authors.
pub fn list_authors(repo_path: &str, limit: usize) -> Result<Vec<String>, String> {
    let repo = Repository::open(repo_path).map_err(|e| format!("Failed to open repo: {e}"))?;

    let mut revwalk = repo.revwalk().map_err(|e| format!("Revwalk error: {e}"))?;
    revwalk.set_sorting(Sort::TIME).ok();
    revwalk.push_glob("refs/heads/*").ok();
    revwalk.push_glob("refs/remotes/*").ok();

    let mut authors = HashSet::new();
    let mut count = 0;
    for oid_result in revwalk {
        if count >= limit {
            break;
        }
        if let Ok(oid) = oid_result {
            if let Ok(commit) = repo.find_commit(oid) {
                if let Some(name) = commit.author().name() {
                    authors.insert(name.to_string());
                }
                count += 1;
            }
        }
    }
    let mut result: Vec<_> = authors.into_iter().collect();
    result.sort();
    Ok(result)
}

/// List all tracked files (equivalent to `git ls-files`).
pub fn list_tracked_files(repo_path: &str) -> Result<Vec<String>, String> {
    let repo = Repository::open(repo_path).map_err(|e| format!("Failed to open repo: {e}"))?;

    // Read the HEAD tree recursively
    let head = repo.head().map_err(|e| format!("No HEAD: {e}"))?;
    let tree = head.peel_to_tree().map_err(|e| format!("No tree: {e}"))?;

    let mut files = Vec::new();
    tree.walk(git2::TreeWalkMode::PreOrder, |dir, entry| {
        if entry.kind() == Some(git2::ObjectType::Blob) {
            let path = if dir.is_empty() {
                entry.name().unwrap_or("").to_string()
            } else {
                format!("{}{}", dir, entry.name().unwrap_or(""))
            };
            files.push(path);
        }
        git2::TreeWalkResult::Ok
    })
    .ok();

    Ok(files)
}

/// Detect the default branch (main/master).
pub fn detect_default_branch(repo_path: &str) -> Result<String, String> {
    let repo = Repository::open(repo_path).map_err(|e| format!("Failed to open repo: {e}"))?;

    // Strategy 1: Check origin/HEAD symbolic ref
    if let Ok(reference) = repo.find_reference("refs/remotes/origin/HEAD") {
        if let Ok(resolved) = reference.resolve() {
            if let Some(name) = resolved.shorthand() {
                return Ok(name.to_string());
            }
        }
        // Try symbolic target
        if let Some(target) = reference.symbolic_target() {
            // refs/remotes/origin/master → origin/master
            if target.starts_with("refs/remotes/") {
                return Ok(target.trim_start_matches("refs/remotes/").to_string());
            }
        }
    }

    // Strategy 2: Try common remote tracking branches
    for name in &["origin/master", "origin/main"] {
        if repo.find_reference(&format!("refs/remotes/{name}")).is_ok() {
            return Ok(name.to_string());
        }
    }

    // Strategy 3: Try local branches
    for name in &["master", "main"] {
        if repo.find_branch(name, BranchType::Local).is_ok() {
            return Ok(name.to_string());
        }
    }

    // Fallback
    Ok("HEAD".to_string())
}

// ═══════════════════════════════════════════════════════════
// IDEA-Style Graph Layout Engine
// ═══════════════════════════════════════════════════════════

/// A line segment within a single row (pixel coordinates relative to row origin).
#[derive(Serialize, Clone)]
pub struct PrintSegment {
    pub x_top: f32,
    pub y_top: f32,
    pub x_bottom: f32,
    pub y_bottom: f32,
    pub color_index: i32,
}

/// Arrow indicator for long-span branch lines.
#[derive(Serialize, Clone)]
pub struct ArrowElement {
    pub x: f32,
    pub y: f32,
    pub color_index: i32,
    pub is_down: bool,
}

/// Input commit entry for layout computation (deserialized from graph_log JSON).
#[derive(Deserialize)]
pub struct LayoutInput {
    pub hash: String,
    pub parents: String,
    pub short_hash: String,
    pub refs: String,
    pub message: String,
    pub author: String,
    pub date_timestamp: i64,
}

/// Fully computed graph row with layout data.
#[derive(Serialize)]
pub struct GraphRow {
    pub hash: String,
    pub short_hash: String,
    pub message: String,
    pub author: String,
    pub date_timestamp: i64,
    pub refs: String,
    pub parents: String,
    pub node_column: i32,
    pub color_index: i32,
    pub segments: Vec<PrintSegment>,
    pub arrows: Vec<ArrowElement>,
}

/// Layout parameters passed from Swift.
pub struct LayoutParams {
    pub lane_width: f32,
    pub row_height: f32,
    pub show_long_edges: bool,
}

/// Compute the full IDEA-style graph layout.
///
/// Takes commit entries + main chain hashes + rendering params,
/// returns fully laid-out `GraphRow` entries with segments and arrows.
pub fn compute_graph_layout(
    commits: &[LayoutInput],
    main_chain: &HashSet<String>,
    params: &LayoutParams,
) -> Vec<GraphRow> {
    if commits.is_empty() {
        return Vec::new();
    }

    let n = commits.len();
    let lw = params.lane_width;
    let rh = params.row_height;

    // Derive mode-specific constants (from IDEA's PrintElementGeneratorImpl.kt)
    let long_edge_size: i32 = if params.show_long_edges { 1000 } else { 30 };
    let visible_part_size: i32 = if params.show_long_edges { 250 } else { 1 };
    let edge_with_arrow_size: i32 = if params.show_long_edges { 30 } else { i32::MAX };

    // Parse parent hashes for each commit
    let parent_lists: Vec<Vec<String>> = commits
        .iter()
        .map(|c| {
            if c.parents.is_empty() {
                Vec::new()
            } else {
                c.parents.split(' ').map(|s| s.to_string()).collect()
            }
        })
        .collect();

    // Build hash → row index map
    let mut hash_to_row: HashMap<&str, usize> = HashMap::with_capacity(n);
    for (i, c) in commits.iter().enumerate() {
        hash_to_row.insert(&c.hash, i);
    }

    // ── Phase 1: DFS Layout Index Assignment ──
    // Matches IDEA's GraphLayoutBuilder.build() exactly.
    let layout_index = assign_layout_indices(n, &parent_lists, &hash_to_row);

    // ── Color Assignment ──
    // layoutIndex → color mapping. Main chain gets color 0.
    let mut li_to_color: HashMap<i32, i32> = HashMap::new();
    let mut next_color: i32 = 1;
    let mut node_colors: Vec<i32> = Vec::with_capacity(n);

    for i in 0..n {
        let li = layout_index[i];
        let is_main = main_chain.contains(&commits[i].hash);
        let ci = if is_main {
            0
        } else if let Some(&c) = li_to_color.get(&li) {
            c
        } else {
            let c = next_color;
            next_color += 1;
            li_to_color.insert(li, c);
            c
        };
        node_colors.push(ci);
    }

    // ── Build Edge List ──
    struct EdgeInfo {
        child_row: usize,
        parent_row: usize,
        _parent_index: usize,
        up_li: i32,
        down_li: i32,
        color_index: i32,
    }
    let mut all_edges: Vec<EdgeInfo> = Vec::new();

    for (child_row, parents) in parent_lists.iter().enumerate() {
        for (pi, parent_hash) in parents.iter().enumerate() {
            let parent_row = match hash_to_row.get(parent_hash.as_str()) {
                Some(&pr) => pr,
                None => continue,
            };
            if parent_row <= child_row {
                continue;
            }

            let child_li = layout_index[child_row];
            let parent_li = if parent_row < n {
                layout_index[parent_row]
            } else {
                child_li
            };

            // Color: first-parent edges inherit child color;
            // merge edges (2nd+ parent) use parent color
            let ci = if pi == 0 {
                node_colors[child_row]
            } else if parent_row < n {
                node_colors[parent_row]
            } else {
                node_colors[child_row]
            };

            all_edges.push(EdgeInfo {
                child_row,
                parent_row,
                _parent_index: pi,
                up_li: child_li,
                down_li: parent_li,
                color_index: ci,
            });
        }
    }

    // ── Phase 2: Column Positions per Row ──
    // Sweep through rows, maintaining active edges.
    // At each row: sort(node + active_edges) by IDEA comparator → position = column.

    // Group edges by their first intermediate row
    let mut edges_by_start: HashMap<usize, Vec<usize>> = HashMap::new();
    for (ei, edge) in all_edges.iter().enumerate() {
        let first_intermediate = edge.child_row + 1;
        let last_intermediate = edge.parent_row.saturating_sub(1).min(n - 1);
        if first_intermediate <= last_intermediate {
            edges_by_start
                .entry(first_intermediate)
                .or_default()
                .push(ei);
        }
    }

    let mut active_edge_indices: HashSet<usize> = HashSet::new();
    let mut node_columns: Vec<i32> = vec![0; n];
    let mut edge_column_at_row: Vec<HashMap<usize, i32>> = vec![HashMap::new(); n];

    for row in 0..n {
        // Add edges whose first intermediate row is this row
        if let Some(new_edges) = edges_by_start.get(&row) {
            for &ei in new_edges {
                active_edge_indices.insert(ei);
            }
        }

        // Build sorted elements for this row using IDEA's comparator
        #[derive(Clone)]
        struct RowElement {
            is_node: bool,
            edge_index: usize,
            up_li: i32,
            down_li: i32,
            up_row: i32,
            down_row: i32,
        }

        // IDEA's compare2(edge, node): positive means edge goes RIGHT of node
        fn compare2(e: &RowElement, n_elem: &RowElement) -> i32 {
            let max_edge_li = e.up_li.max(e.down_li);
            let node_li = n_elem.up_li;
            if max_edge_li != node_li {
                return max_edge_li - node_li;
            }
            e.up_row - n_elem.up_row
        }

        fn compare_elements(lhs: &RowElement, rhs: &RowElement) -> i32 {
            if !lhs.is_node && !rhs.is_node {
                // Edge vs Edge
                if lhs.up_row == rhs.up_row {
                    if lhs.down_row < rhs.down_row {
                        let vn = RowElement {
                            is_node: true,
                            edge_index: 0,
                            up_li: lhs.down_li,
                            down_li: lhs.down_li,
                            up_row: lhs.down_row,
                            down_row: lhs.down_row,
                        };
                        return -compare2(rhs, &vn);
                    } else {
                        let vn = RowElement {
                            is_node: true,
                            edge_index: 0,
                            up_li: rhs.down_li,
                            down_li: rhs.down_li,
                            up_row: rhs.down_row,
                            down_row: rhs.down_row,
                        };
                        return compare2(lhs, &vn);
                    }
                }
                if lhs.up_row < rhs.up_row {
                    let vn = RowElement {
                        is_node: true,
                        edge_index: 0,
                        up_li: rhs.up_li,
                        down_li: rhs.up_li,
                        up_row: rhs.up_row,
                        down_row: rhs.up_row,
                    };
                    return compare2(lhs, &vn);
                } else {
                    let vn = RowElement {
                        is_node: true,
                        edge_index: 0,
                        up_li: lhs.up_li,
                        down_li: lhs.up_li,
                        up_row: lhs.up_row,
                        down_row: lhs.up_row,
                    };
                    return -compare2(rhs, &vn);
                }
            }
            if !lhs.is_node && rhs.is_node {
                return compare2(lhs, rhs);
            }
            if lhs.is_node && !rhs.is_node {
                return -compare2(rhs, lhs);
            }
            0
        }

        let node_li = layout_index[row];
        let mut elements: Vec<RowElement> = Vec::new();
        elements.push(RowElement {
            is_node: true,
            edge_index: 0,
            up_li: node_li,
            down_li: node_li,
            up_row: row as i32,
            down_row: row as i32,
        });

        for &ei in &active_edge_indices {
            let e = &all_edges[ei];
            let clamped_pr = e.parent_row.min(n - 1);
            if !is_edge_visible_in_row(
                e.child_row as i32,
                clamped_pr as i32,
                row as i32,
                long_edge_size,
                visible_part_size,
                edge_with_arrow_size,
            ) {
                continue;
            }
            elements.push(RowElement {
                is_node: false,
                edge_index: ei,
                up_li: e.up_li,
                down_li: e.down_li,
                up_row: e.child_row as i32,
                down_row: e.parent_row as i32,
            });
        }

        elements.sort_by(|a, b| {
            let cmp = compare_elements(a, b);
            cmp.cmp(&0)
        });

        for (col, elem) in elements.iter().enumerate() {
            if elem.is_node {
                node_columns[row] = col as i32;
            } else {
                edge_column_at_row[row].insert(elem.edge_index, col as i32);
            }
        }

        // Remove edges whose last intermediate row is this row
        active_edge_indices.retain(|&ei| {
            let last_interm = all_edges[ei].parent_row.saturating_sub(1).min(n - 1);
            row < last_interm
        });
    }

    // ── Phase 3: Generate Segments and Arrows ──
    let x_pos = |col: i32| -> f32 { col as f32 * lw + lw / 2.0 + 4.0 };
    let approach_len: f32 = 8.0;

    // Pre-allocate per-row segment/arrow storage
    let mut row_segments: Vec<Vec<PrintSegment>> = vec![Vec::new(); n];
    let mut row_arrows: Vec<Vec<ArrowElement>> = vec![Vec::new(); n];

    for (ei, edge) in all_edges.iter().enumerate() {
        let ci = edge.color_index;
        let clamped_parent = edge.parent_row.min(n - 1);
        let span = clamped_parent as i32 - edge.child_row as i32;
        if span <= 0 {
            continue;
        }

        // Build anchor list: (row, x) — only visible rows
        let mut anchors: Vec<(usize, f32)> = Vec::new();
        anchors.push((edge.child_row, x_pos(node_columns[edge.child_row])));

        for (r, row_edge_columns) in edge_column_at_row
            .iter()
            .enumerate()
            .take(clamped_parent)
            .skip(edge.child_row + 1)
        {
            if !is_edge_visible_in_row(
                edge.child_row as i32,
                clamped_parent as i32,
                r as i32,
                long_edge_size,
                visible_part_size,
                edge_with_arrow_size,
            ) {
                continue;
            }
            let col = row_edge_columns
                .get(&ei)
                .copied()
                .unwrap_or(node_columns[edge.child_row]);
            anchors.push((r, x_pos(col)));
        }
        anchors.push((clamped_parent, x_pos(node_columns[clamped_parent])));

        // Pre-compute arrow rows
        let mut down_arrow_rows: HashSet<usize> = HashSet::new();
        let mut up_arrow_rows: HashSet<usize> = HashSet::new();
        if span >= long_edge_size {
            down_arrow_rows.insert(edge.child_row + visible_part_size as usize);
            if clamped_parent >= visible_part_size as usize {
                up_arrow_rows.insert(clamped_parent - visible_part_size as usize);
            }
        }
        if span >= edge_with_arrow_size {
            down_arrow_rows.insert(edge.child_row + 1);
            if clamped_parent >= 1 {
                up_arrow_rows.insert(clamped_parent - 1);
            }
        }

        // Generate half-row segments between consecutive anchors
        for ai in 0..anchors.len().saturating_sub(1) {
            let (row_a, x_a) = anchors[ai];
            let (row_b, x_b) = anchors[ai + 1];
            if row_a >= n {
                continue;
            }

            if row_b == row_a + 1 {
                let x_mid = (x_a + x_b) / 2.0;
                let is_diagonal = (x_a - x_b).abs() > 0.5;

                // Bottom half of row_a
                if up_arrow_rows.contains(&row_a) && is_diagonal {
                    row_segments[row_a].push(PrintSegment {
                        x_top: x_a,
                        y_top: 0.0,
                        x_bottom: x_a,
                        y_bottom: approach_len,
                        color_index: ci,
                    });
                    row_segments[row_a].push(PrintSegment {
                        x_top: x_a,
                        y_top: approach_len,
                        x_bottom: x_mid,
                        y_bottom: rh,
                        color_index: ci,
                    });
                } else if up_arrow_rows.contains(&row_a) {
                    row_segments[row_a].push(PrintSegment {
                        x_top: x_a,
                        y_top: 0.0,
                        x_bottom: x_mid,
                        y_bottom: rh,
                        color_index: ci,
                    });
                } else {
                    row_segments[row_a].push(PrintSegment {
                        x_top: x_a,
                        y_top: rh / 2.0,
                        x_bottom: x_mid,
                        y_bottom: rh,
                        color_index: ci,
                    });
                }

                // Top half of row_b
                if row_b < n {
                    if down_arrow_rows.contains(&row_b) && is_diagonal {
                        row_segments[row_b].push(PrintSegment {
                            x_top: x_mid,
                            y_top: 0.0,
                            x_bottom: x_b,
                            y_bottom: rh - approach_len,
                            color_index: ci,
                        });
                        row_segments[row_b].push(PrintSegment {
                            x_top: x_b,
                            y_top: rh - approach_len,
                            x_bottom: x_b,
                            y_bottom: rh,
                            color_index: ci,
                        });
                    } else if down_arrow_rows.contains(&row_b) {
                        row_segments[row_b].push(PrintSegment {
                            x_top: x_mid,
                            y_top: 0.0,
                            x_bottom: x_b,
                            y_bottom: rh,
                            color_index: ci,
                        });
                    } else {
                        row_segments[row_b].push(PrintSegment {
                            x_top: x_mid,
                            y_top: 0.0,
                            x_bottom: x_b,
                            y_bottom: rh / 2.0,
                            color_index: ci,
                        });
                    }
                }
            }
            // else: gap in visibility (long edge break) — no segments drawn
        }

        // Add arrow indicators — dual-rule system
        // Rule 1: Long-edge break arrows (span >= longEdgeSize)
        if span >= long_edge_size {
            let down_row = edge.child_row + visible_part_size as usize;
            if down_row < n {
                let col = edge_column_at_row[down_row]
                    .get(&ei)
                    .copied()
                    .unwrap_or(node_columns[edge.child_row]);
                row_arrows[down_row].push(ArrowElement {
                    x: x_pos(col),
                    y: rh,
                    color_index: ci,
                    is_down: true,
                });
            }
            if clamped_parent >= visible_part_size as usize {
                let up_row = clamped_parent - visible_part_size as usize;
                if up_row < n {
                    let col = edge_column_at_row[up_row]
                        .get(&ei)
                        .copied()
                        .unwrap_or(node_columns[clamped_parent]);
                    row_arrows[up_row].push(ArrowElement {
                        x: x_pos(col),
                        y: 0.0,
                        color_index: ci,
                        is_down: false,
                    });
                }
            }
        }
        // Rule 2: Visible-edge arrows (span >= edgeWithArrowSize)
        if span >= edge_with_arrow_size {
            let down_row = edge.child_row + 1;
            if down_row < n {
                let col = edge_column_at_row[down_row]
                    .get(&ei)
                    .copied()
                    .unwrap_or(node_columns[edge.child_row]);
                row_arrows[down_row].push(ArrowElement {
                    x: x_pos(col),
                    y: rh,
                    color_index: ci,
                    is_down: true,
                });
            }
            if clamped_parent >= 1 {
                let up_row = clamped_parent - 1;
                if up_row < n {
                    let col = edge_column_at_row[up_row]
                        .get(&ei)
                        .copied()
                        .unwrap_or(node_columns[clamped_parent]);
                    row_arrows[up_row].push(ArrowElement {
                        x: x_pos(col),
                        y: 0.0,
                        color_index: ci,
                        is_down: false,
                    });
                }
            }
        }
    }

    // ── Build output ──
    let mut result: Vec<GraphRow> = Vec::with_capacity(n);
    for i in 0..n {
        result.push(GraphRow {
            hash: commits[i].hash.clone(),
            short_hash: commits[i].short_hash.clone(),
            message: commits[i].message.clone(),
            author: commits[i].author.clone(),
            date_timestamp: commits[i].date_timestamp,
            refs: commits[i].refs.clone(),
            parents: commits[i].parents.clone(),
            node_column: node_columns[i],
            color_index: node_colors[i],
            segments: std::mem::take(&mut row_segments[i]),
            arrows: std::mem::take(&mut row_arrows[i]),
        });
    }
    result
}

// ── Phase 1 helper: DFS layout index assignment ──

fn assign_layout_indices(
    n: usize,
    parent_lists: &[Vec<String>],
    hash_to_row: &HashMap<&str, usize>,
) -> Vec<i32> {
    let mut layout_index = vec![0i32; n];
    let mut current_li: i32 = 1;

    // Identify heads: nodes not referenced as parent by any other node
    let mut parent_set: HashSet<usize> = HashSet::new();
    for parents in parent_lists {
        for p in parents {
            if let Some(&pr) = hash_to_row.get(p.as_str()) {
                parent_set.insert(pr);
            }
        }
    }
    let mut heads: Vec<usize> = Vec::new();
    for i in 0..n {
        if !parent_set.contains(&i) {
            heads.push(i);
        }
    }
    heads.sort();

    // DFS walk from each head
    let dfs_walk = |head: usize, li: &mut Vec<i32>, current: &mut i32| {
        if li[head] != 0 {
            return;
        }
        let mut stack = vec![head];
        while let Some(&cur) = stack.last() {
            let first_visit = li[cur] == 0;
            if first_visit {
                li[cur] = *current;
            }
            // Find first unvisited parent
            let mut next_node: Option<usize> = None;
            for p in &parent_lists[cur] {
                if let Some(&pr) = hash_to_row.get(p.as_str()) {
                    if li[pr] == 0 {
                        next_node = Some(pr);
                        break;
                    }
                }
            }
            if let Some(next) = next_node {
                stack.push(next);
            } else {
                if first_visit {
                    *current += 1;
                }
                stack.pop();
            }
        }
    };

    for &head in &heads {
        dfs_walk(head, &mut layout_index, &mut current_li);
    }
    // Assign any remaining disconnected nodes
    for i in 0..n {
        if layout_index[i] == 0 {
            dfs_walk(i, &mut layout_index, &mut current_li);
        }
    }

    layout_index
}

// ── Visibility helpers ──

fn is_edge_visible_in_row(
    child_row: i32,
    parent_row: i32,
    row: i32,
    long_edge_size: i32,
    visible_part_size: i32,
    edge_with_arrow_size: i32,
) -> bool {
    let span = parent_row - child_row;
    if span >= long_edge_size {
        let up_offset = row - child_row;
        let down_offset = parent_row - row;
        return up_offset <= visible_part_size || down_offset <= visible_part_size;
    }
    if span >= edge_with_arrow_size {
        let up_offset = row - child_row;
        let down_offset = parent_row - row;
        return up_offset <= 1 || down_offset <= 1;
    }
    true
}
