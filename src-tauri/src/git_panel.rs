use pier_core::git_graph::{self, GraphFilter, LayoutInput, LayoutParams};
use pier_core::services::git::{BlameLine, GitClient};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitPanelState {
    pub repo_path: String,
    pub current_branch: String,
    pub tracking_branch: String,
    pub ahead_count: i32,
    pub behind_count: i32,
    pub staged_files: Vec<GitPanelFile>,
    pub unstaged_files: Vec<GitPanelFile>,
    pub total_changes: usize,
    pub conflict_count: usize,
    pub working_tree_clean: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GitPanelFile {
    pub path: String,
    pub file_name: String,
    pub status: String,
    pub staged: bool,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitGraphMetadata {
    pub branches: Vec<String>,
    pub authors: Vec<String>,
    pub repo_files: Vec<String>,
    pub git_user_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitGraphRowView {
    pub hash: String,
    pub short_hash: String,
    pub message: String,
    pub author: String,
    pub date_timestamp: i64,
    pub refs: String,
    pub parents: String,
    pub node_column: i32,
    pub color_index: i32,
    pub segments: Vec<GitGraphSegmentView>,
    pub arrows: Vec<GitGraphArrowView>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitGraphSegmentView {
    pub x_top: f32,
    pub y_top: f32,
    pub x_bottom: f32,
    pub y_bottom: f32,
    pub color_index: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitGraphArrowView {
    pub x: f32,
    pub y: f32,
    pub color_index: i32,
    pub is_down: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitDetailView {
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    pub date: String,
    pub message: String,
    pub parent_hash: String,
    pub parent_hashes: Vec<String>,
    pub stats: String,
    pub changed_files: Vec<GitCommitChangedFileView>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitChangedFileView {
    pub additions: i32,
    pub deletions: i32,
    pub path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitComparisonFileView {
    pub path: String,
    pub name: String,
    pub dir: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitTagView {
    pub name: String,
    pub hash: String,
    pub timestamp: i64,
    pub message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRemoteView {
    pub name: String,
    pub fetch_url: String,
    pub push_url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitConfigEntryView {
    pub key: String,
    pub value: String,
    pub scope: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRebasePlanView {
    pub in_progress: bool,
    pub items: Vec<GitRebaseItemView>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GitRebaseItemView {
    pub id: String,
    pub action: String,
    pub hash: String,
    pub short_hash: String,
    pub message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitSubmoduleView {
    pub path: String,
    pub commit_hash: String,
    pub short_hash: String,
    pub status: String,
    pub status_symbol: String,
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GitConflictHunkView {
    pub ours_lines: Vec<String>,
    pub theirs_lines: Vec<String>,
    pub resolution: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitConflictFileView {
    pub name: String,
    pub path: String,
    pub conflict_count: usize,
    pub conflicts: Vec<GitConflictHunkView>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitBlameLineView {
    pub line_number: u32,
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    pub timestamp: i64,
    pub date: String,
    pub content: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitGraphHistoryParams {
    pub path: Option<String>,
    pub limit: Option<usize>,
    pub skip: Option<usize>,
    pub branch: Option<String>,
    pub author: Option<String>,
    pub search_text: Option<String>,
    pub first_parent: Option<bool>,
    pub no_merges: Option<bool>,
    pub after_timestamp: Option<i64>,
    pub paths: Option<Vec<String>>,
    pub topo_order: Option<bool>,
    pub show_long_edges: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitConflictMarkResolvedParams {
    pub path: Option<String>,
    pub file_path: String,
    pub hunks: Vec<GitConflictHunkView>,
}

fn workspace_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn resolve_existing_path(path: Option<String>) -> PathBuf {
    path.map(PathBuf::from)
        .filter(|candidate| candidate.exists())
        .unwrap_or_else(workspace_root)
}

fn open_git_client(path: Option<String>) -> Result<GitClient, String> {
    let target = resolve_existing_path(path);
    let target_str = target.display().to_string();
    GitClient::open(&target_str).map_err(|error| error.to_string())
}

fn repo_root(path: Option<String>) -> Result<PathBuf, String> {
    open_git_client(path).map(|client| client.repo_path().to_path_buf())
}

fn run_git_at(repo_path: &Path, args: &[&str]) -> Result<String, String> {
    run_git_at_with_env(repo_path, args, &[])
}

fn run_git_at_with_env(
    repo_path: &Path,
    args: &[&str],
    env: &[(&str, String)],
) -> Result<String, String> {
    let mut command = Command::new("git");
    command.current_dir(repo_path);
    command.args(args);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command.env("GIT_TERMINAL_PROMPT", "0");
    for (key, value) in env {
        command.env(key, value);
    }

    let output = command
        .output()
        .map_err(|error| format!("failed to run git {}: {}", args.join(" "), error))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        if output.status.code() == Some(1) && stderr.trim().is_empty() {
            return Ok(stdout);
        }
        let detail = stderr.trim();
        return Err(if detail.is_empty() {
            format!("git {} failed", args.join(" "))
        } else {
            format!("git {} failed: {}", args.join(" "), detail)
        });
    }

    Ok(stdout)
}

fn git_path_for_repo(repo_path: &Path, relative: &str) -> Result<PathBuf, String> {
    let resolved = run_git_at(repo_path, &["rev-parse", "--git-path", relative])?;
    Ok(PathBuf::from(resolved.trim()))
}

fn parse_commit_detail(
    meta_output: &str,
    stats_output: &str,
    numstat_output: &str,
    parents_output: &str,
) -> GitCommitDetailView {
    let separator = '\u{001f}';
    let mut detail = GitCommitDetailView {
        hash: String::new(),
        short_hash: String::new(),
        author: String::new(),
        date: String::new(),
        message: String::new(),
        parent_hash: String::new(),
        parent_hashes: Vec::new(),
        stats: String::new(),
        changed_files: Vec::new(),
    };

    let parts: Vec<&str> = meta_output.splitn(5, separator).collect();
    if parts.len() == 5 {
        detail.hash = parts[0].trim().to_string();
        detail.short_hash = parts[1].trim().to_string();
        detail.author = parts[2].trim().to_string();
        detail.date = parts[3].trim().to_string();
        detail.message = parts[4].trim().to_string();
    }

    let parent_tokens: Vec<&str> = parents_output.split_whitespace().collect();
    if parent_tokens.len() > 1 {
        detail.parent_hashes = parent_tokens[1..].iter().map(|value| (*value).to_string()).collect();
        detail.parent_hash = detail.parent_hashes.first().cloned().unwrap_or_default();
    }

    for line in stats_output.lines().rev() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            detail.stats = trimmed.to_string();
            break;
        }
    }

    for line in numstat_output.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 3 {
            continue;
        }
        let additions = if parts[0] == "-" { 0 } else { parts[0].parse().unwrap_or(0) };
        let deletions = if parts[1] == "-" { 0 } else { parts[1].parse().unwrap_or(0) };
        let path = parts[2..].join("\t").trim().to_string();
        detail.changed_files.push(GitCommitChangedFileView {
            additions,
            deletions,
            path,
        });
    }

    detail
}

fn parse_rebase_line(line: &str) -> Option<GitRebaseItemView> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "noop" {
        return None;
    }

    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }

    let action = parts[0].trim().to_string();
    let hash = parts[1].trim().to_string();
    let message = parts[2..].join(" ").trim().to_string();

    Some(GitRebaseItemView {
        id: hash.clone(),
        action,
        hash: hash.clone(),
        short_hash: hash.chars().take(7).collect(),
        message,
    })
}

fn parse_gitmodules_urls(repo_path: &Path) -> std::collections::HashMap<String, String> {
    let mut urls = std::collections::HashMap::new();
    let gitmodules = repo_path.join(".gitmodules");
    let Ok(content) = fs::read_to_string(gitmodules) else {
        return urls;
    };

    let mut current_path = String::new();
    let mut current_url = String::new();
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') {
            if !current_path.is_empty() {
                urls.insert(current_path.clone(), current_url.clone());
            }
            current_path.clear();
            current_url.clear();
            continue;
        }

        if let Some((_, value)) = line.split_once('=') {
            if line.starts_with("path") {
                current_path = value.trim().to_string();
            } else if line.starts_with("url") {
                current_url = value.trim().to_string();
            }
        }
    }

    if !current_path.is_empty() {
        urls.insert(current_path, current_url);
    }

    urls
}

fn parse_conflict_hunks(content: &str) -> Vec<GitConflictHunkView> {
    let lines: Vec<&str> = content.split('\n').collect();
    let mut hunks = Vec::new();
    let mut index = 0usize;

    while index < lines.len() {
        if !lines[index].starts_with("<<<<<<<") {
            index += 1;
            continue;
        }

        let mut ours_lines = Vec::new();
        let mut theirs_lines = Vec::new();
        index += 1;

        while index < lines.len() && !lines[index].starts_with("=======") {
            ours_lines.push(lines[index].to_string());
            index += 1;
        }
        if index < lines.len() {
            index += 1;
        }

        while index < lines.len() && !lines[index].starts_with(">>>>>>>") {
            theirs_lines.push(lines[index].to_string());
            index += 1;
        }

        hunks.push(GitConflictHunkView {
            ours_lines,
            theirs_lines,
            resolution: String::new(),
        });

        if index < lines.len() {
            index += 1;
        }
    }

    hunks
}

fn write_resolved_conflict_file(
    repo_path: &Path,
    file_path: &str,
    hunks: &[GitConflictHunkView],
    default_resolution: &str,
) -> Result<(), String> {
    let absolute_path = repo_path.join(file_path);
    let content = fs::read_to_string(&absolute_path)
        .map_err(|error| format!("Failed to read {}: {}", absolute_path.display(), error))?;
    let lines: Vec<&str> = content.split('\n').collect();
    let mut result = Vec::new();
    let mut line_index = 0usize;
    let mut hunk_index = 0usize;

    while line_index < lines.len() {
        if !lines[line_index].starts_with("<<<<<<<") {
            result.push(lines[line_index].to_string());
            line_index += 1;
            continue;
        }

        let mut ours_lines = Vec::new();
        let mut theirs_lines = Vec::new();
        line_index += 1;
        while line_index < lines.len() && !lines[line_index].starts_with("=======") {
            ours_lines.push(lines[line_index].to_string());
            line_index += 1;
        }
        if line_index < lines.len() {
            line_index += 1;
        }
        while line_index < lines.len() && !lines[line_index].starts_with(">>>>>>>") {
            theirs_lines.push(lines[line_index].to_string());
            line_index += 1;
        }

        let resolution = hunks
            .get(hunk_index)
            .map(|hunk| hunk.resolution.trim())
            .filter(|value| !value.is_empty())
            .unwrap_or(default_resolution);

        match resolution {
            "theirs" => result.extend(theirs_lines),
            "both" => {
                result.extend(ours_lines);
                result.extend(theirs_lines);
            }
            _ => result.extend(ours_lines),
        }

        hunk_index += 1;
        if line_index < lines.len() {
            line_index += 1;
        }
    }

    fs::write(&absolute_path, result.join("\n"))
        .map_err(|error| format!("Failed to write {}: {}", absolute_path.display(), error))
}

fn unique_temp_path(prefix: &str, extension: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("{prefix}-{stamp}.{extension}"))
}

fn create_sequence_editor_script(todo_path: &Path) -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        let script_path = unique_temp_path("pierx-sequence-editor", "cmd");
        let content = format!(
            "@echo off\r\ncopy /Y \"{}\" \"%~1\" >NUL\r\n",
            todo_path.display()
        );
        fs::write(&script_path, content)
            .map_err(|error| format!("Failed to write {}: {}", script_path.display(), error))?;
        Ok(script_path)
    }

    #[cfg(not(windows))]
    {
        let script_path = unique_temp_path("pierx-sequence-editor", "sh");
        let content = format!("#!/bin/sh\ncat \"{}\" > \"$1\"\n", todo_path.display());
        fs::write(&script_path, content)
            .map_err(|error| format!("Failed to write {}: {}", script_path.display(), error))?;
        #[cfg(unix)]
        {
            let mut permissions = fs::metadata(&script_path)
                .map_err(|error| format!("Failed to stat {}: {}", script_path.display(), error))?
                .permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(&script_path, permissions).map_err(|error| {
                format!("Failed to set permissions on {}: {}", script_path.display(), error)
            })?;
        }
        Ok(script_path)
    }
}

fn format_blame_date(timestamp: i64) -> String {
    if timestamp <= 0 {
        return String::new();
    }
    let output = Command::new("date")
        .args(["-r", &timestamp.to_string(), "+%Y-%m-%d %H:%M"])
        .output();
    match output {
        Ok(result) if result.status.success() => {
            String::from_utf8_lossy(&result.stdout).trim().to_string()
        }
        _ => String::new(),
    }
}

fn map_git_panel_file(change: pier_core::services::git::GitFileChange) -> GitPanelFile {
    let file_name = Path::new(&change.path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(&change.path)
        .to_string();

    GitPanelFile {
        path: change.path,
        file_name,
        status: change.status.code().to_string(),
        staged: change.staged,
        additions: 0,
        deletions: 0,
    }
}

fn parse_numstat(output: &str) -> std::collections::HashMap<String, (u32, u32)> {
    let mut map = std::collections::HashMap::new();
    for line in output.lines() {
        let mut parts = line.splitn(3, '\t');
        let adds = parts.next().unwrap_or("0");
        let dels = parts.next().unwrap_or("0");
        let path = match parts.next() {
            Some(value) => value.trim().to_string(),
            None => continue,
        };
        if path.is_empty() {
            continue;
        }
        let add_count = adds.parse::<u32>().unwrap_or(0);
        let del_count = dels.parse::<u32>().unwrap_or(0);
        map.insert(path, (add_count, del_count));
    }
    map
}

fn count_file_lines(repo_path: &Path, rel_path: &str) -> u32 {
    let full = repo_path.join(rel_path);
    match fs::read(&full) {
        Ok(bytes) => {
            if bytes.is_empty() {
                return 0;
            }
            let mut count = bytes.iter().filter(|b| **b == b'\n').count() as u32;
            if !bytes.ends_with(b"\n") {
                count += 1;
            }
            count
        }
        Err(_) => 0,
    }
}

fn annotate_panel_file(
    repo_path: &Path,
    mut file: GitPanelFile,
    staged_map: &std::collections::HashMap<String, (u32, u32)>,
    unstaged_map: &std::collections::HashMap<String, (u32, u32)>,
) -> GitPanelFile {
    let source = if file.staged { staged_map } else { unstaged_map };
    if let Some((adds, dels)) = source.get(&file.path) {
        file.additions = *adds;
        file.deletions = *dels;
    } else if !file.staged && file.status == "?" {
        file.additions = count_file_lines(repo_path, &file.path);
    }
    file
}

#[tauri::command]
pub fn git_init_repo(path: Option<String>) -> Result<String, String> {
    let repo_path = resolve_existing_path(path);
    run_git_at(&repo_path, &["init"])
}

#[tauri::command]
pub fn git_panel_state(path: Option<String>) -> Result<GitPanelState, String> {
    let client = open_git_client(path)?;
    let branch = client.branch_info().map_err(|error| error.to_string())?;
    let changes = client.status().map_err(|error| error.to_string())?;
    let repo_path = client.repo_path().to_path_buf();

    let staged_numstat = run_git_at(&repo_path, &["diff", "--cached", "--numstat"])
        .map(|out| parse_numstat(&out))
        .unwrap_or_default();
    let unstaged_numstat = run_git_at(&repo_path, &["diff", "--numstat"])
        .map(|out| parse_numstat(&out))
        .unwrap_or_default();

    let staged_files: Vec<GitPanelFile> = changes
        .iter()
        .filter(|change| change.staged)
        .cloned()
        .map(map_git_panel_file)
        .map(|file| annotate_panel_file(&repo_path, file, &staged_numstat, &unstaged_numstat))
        .collect();
    let unstaged_files: Vec<GitPanelFile> = changes
        .iter()
        .filter(|change| !change.staged)
        .cloned()
        .map(map_git_panel_file)
        .map(|file| annotate_panel_file(&repo_path, file, &staged_numstat, &unstaged_numstat))
        .collect();
    let conflict_count = unstaged_files.iter().filter(|file| file.status == "U").count();

    Ok(GitPanelState {
        repo_path: client.repo_path().display().to_string(),
        current_branch: branch.name,
        tracking_branch: branch.tracking,
        ahead_count: branch.ahead,
        behind_count: branch.behind,
        total_changes: staged_files.len() + unstaged_files.len(),
        conflict_count,
        working_tree_clean: staged_files.is_empty() && unstaged_files.is_empty(),
        staged_files,
        unstaged_files,
    })
}

#[tauri::command]
pub fn git_commit_and_push(
    path: Option<String>,
    message: String,
    signoff: Option<bool>,
    amend: Option<bool>,
    sign: Option<bool>,
) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return Err(String::from("commit message cannot be empty"));
    }
    let mut args: Vec<&str> = vec!["commit"];
    if signoff.unwrap_or(false) {
        args.push("--signoff");
    }
    if amend.unwrap_or(false) {
        args.push("--amend");
    }
    if sign.unwrap_or(false) {
        args.push("-S");
    }
    args.extend_from_slice(&["-m", trimmed]);
    run_git_at(&repo_path, &args)?;
    run_git_at(&repo_path, &["push"])
}

#[tauri::command]
pub fn git_graph_metadata(path: Option<String>) -> Result<GitGraphMetadata, String> {
    let client = open_git_client(path.clone())?;
    let repo_path = client.repo_path().display().to_string();
    let branches = git_graph::list_branches(&repo_path)?;
    let authors = git_graph::list_authors(&repo_path, 96)?;
    let repo_files = git_graph::list_tracked_files(&repo_path)?;
    let git_user_name = run_git_at(client.repo_path(), &["config", "user.name"])
        .map(|value| value.trim().to_string())
        .unwrap_or_default();

    Ok(GitGraphMetadata {
        branches,
        authors,
        repo_files,
        git_user_name,
    })
}

#[tauri::command]
pub fn git_graph_history(params: GitGraphHistoryParams) -> Result<Vec<GitGraphRowView>, String> {
    let client = open_git_client(params.path.clone())?;
    let repo_path = client.repo_path().display().to_string();
    let current_branch = client
        .branch_info()
        .map(|info| info.name)
        .unwrap_or_else(|_| String::from("HEAD"));
    // Only use an explicit branch filter when the user picked one. Falling
    // back to `current_branch` here silently turned the graph into a
    // single-lane first-parent view of main for every repo with no
    // user-set filter — which is why branches never appeared.
    let explicit_branch = params
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(String::from);

    let filter = GraphFilter {
        branch: explicit_branch.clone(),
        author: params
            .author
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(String::from),
        search_text: params
            .search_text
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(String::from),
        after_timestamp: params.after_timestamp.unwrap_or(0),
        topo_order: params.topo_order.unwrap_or(true),
        first_parent_only: params.first_parent.unwrap_or(false),
        no_merges: params.no_merges.unwrap_or(false),
        paths: params
            .paths
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| !entry.trim().is_empty())
            .collect(),
    };

    let limit = params.limit.unwrap_or(180).clamp(1, 800);
    let skip = params.skip.unwrap_or(0);
    let commits = git_graph::graph_log(&repo_path, limit, skip, &filter)?;

    // Color-0 main chain: prefer the user's selected branch, then the
    // currently-checked-out branch, then auto-detect (main/master/HEAD).
    let main_ref = explicit_branch
        .or_else(|| {
            let cb = current_branch.trim();
            if cb.is_empty() || cb == "HEAD" { None } else { Some(cb.to_string()) }
        })
        .unwrap_or_else(|| git_graph::detect_default_branch(&repo_path).unwrap_or_else(|_| String::from("HEAD")));
    let main_chain: HashSet<String> = git_graph::first_parent_chain(&repo_path, &main_ref, limit)?
        .into_iter()
        .collect();
    let layout_inputs: Vec<LayoutInput> = commits
        .iter()
        .map(|commit| LayoutInput {
            hash: commit.hash.clone(),
            parents: commit.parents.clone(),
            short_hash: commit.short_hash.clone(),
            refs: commit.refs.clone(),
            message: commit.message.clone(),
            author: commit.author.clone(),
            date_timestamp: commit.date_timestamp,
        })
        .collect();
    let rows = git_graph::compute_graph_layout(
        &layout_inputs,
        &main_chain,
        &LayoutParams {
            lane_width: 14.0,
            row_height: 24.0,
            show_long_edges: params.show_long_edges.unwrap_or(true),
        },
    );

    Ok(rows
        .into_iter()
        .map(|row| GitGraphRowView {
            hash: row.hash,
            short_hash: row.short_hash,
            message: row.message,
            author: row.author,
            date_timestamp: row.date_timestamp,
            refs: row.refs,
            parents: row.parents,
            node_column: row.node_column,
            color_index: row.color_index,
            segments: row
                .segments
                .into_iter()
                .map(|segment| GitGraphSegmentView {
                    x_top: segment.x_top,
                    y_top: segment.y_top,
                    x_bottom: segment.x_bottom,
                    y_bottom: segment.y_bottom,
                    color_index: segment.color_index,
                })
                .collect(),
            arrows: row
                .arrows
                .into_iter()
                .map(|arrow| GitGraphArrowView {
                    x: arrow.x,
                    y: arrow.y,
                    color_index: arrow.color_index,
                    is_down: arrow.is_down,
                })
                .collect(),
        })
        .collect())
}

#[tauri::command]
pub fn git_commit_detail(path: Option<String>, hash: String) -> Result<GitCommitDetailView, String> {
    let repo_path = repo_root(path)?;
    let commit_hash = hash.trim();
    if commit_hash.is_empty() {
        return Err(String::from("commit hash cannot be empty"));
    }

    let meta = run_git_at(
        &repo_path,
        &[
            "show",
            "--quiet",
            "--format=%H%x1f%h%x1f%an%x1f%aI%x1f%B",
            commit_hash,
        ],
    )?;
    let stats = run_git_at(&repo_path, &["show", "--shortstat", "--format=", commit_hash]).unwrap_or_default();
    let numstat = run_git_at(&repo_path, &["show", "--numstat", "--format=", commit_hash]).unwrap_or_default();
    let parents = run_git_at(&repo_path, &["rev-list", "--parents", "-n", "1", commit_hash]).unwrap_or_default();
    Ok(parse_commit_detail(&meta, &stats, &numstat, &parents))
}

#[tauri::command]
pub fn git_commit_file_diff(path: Option<String>, hash: String, file_path: String) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let commit_hash = hash.trim();
    let relative_path = file_path.trim();
    if commit_hash.is_empty() || relative_path.is_empty() {
        return Err(String::from("commit hash and file path are required"));
    }

    run_git_at(
        &repo_path,
        &[
            "show",
            "--format=",
            "--patch",
            "-m",
            "--first-parent",
            commit_hash,
            "--",
            relative_path,
        ],
    )
}

#[tauri::command]
pub fn git_comparison_files(path: Option<String>, hash: String) -> Result<Vec<GitComparisonFileView>, String> {
    let repo_path = repo_root(path)?;
    let commit_hash = hash.trim();
    if commit_hash.is_empty() {
        return Err(String::from("commit hash cannot be empty"));
    }
    let output = run_git_at(&repo_path, &["diff", "--name-only", commit_hash, "HEAD"])?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let path = Path::new(entry);
            GitComparisonFileView {
                path: entry.to_string(),
                name: path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or(entry)
                    .to_string(),
                dir: path
                    .parent()
                    .and_then(|value| value.to_str())
                    .unwrap_or("")
                    .to_string(),
            }
        })
        .collect())
}

#[tauri::command]
pub fn git_comparison_diff(path: Option<String>, hash: String, file_path: String) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let commit_hash = hash.trim();
    let relative_path = file_path.trim();
    if commit_hash.is_empty() || relative_path.is_empty() {
        return Err(String::from("commit hash and file path are required"));
    }
    run_git_at(
        &repo_path,
        &["diff", "--stat=0", commit_hash, "HEAD", "--", relative_path],
    )
}

#[tauri::command]
pub fn git_checkout_target(path: Option<String>, target: String, tracking: Option<String>) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let target_ref = target.trim();
    if target_ref.is_empty() {
        return Err(String::from("checkout target cannot be empty"));
    }
    let tracking_ref = tracking.unwrap_or_default();
    if tracking_ref.trim().is_empty() {
        run_git_at(&repo_path, &["checkout", target_ref])
    } else {
        run_git_at(&repo_path, &["checkout", "-b", target_ref, tracking_ref.trim()])
    }
}

#[tauri::command]
pub fn git_create_branch(path: Option<String>, name: String) -> Result<String, String> {
    git_create_branch_at(path, name, None)
}

#[tauri::command]
pub fn git_create_branch_at(path: Option<String>, name: String, start_point: Option<String>) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let branch_name = name.trim();
    if branch_name.is_empty() {
        return Err(String::from("branch name cannot be empty"));
    }
    let start = start_point.unwrap_or_default();
    if start.trim().is_empty() {
        run_git_at(&repo_path, &["branch", branch_name])
    } else {
        run_git_at(&repo_path, &["branch", branch_name, start.trim()])
    }
}

#[tauri::command]
pub fn git_delete_branch(path: Option<String>, name: String) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let branch_name = name.trim();
    if branch_name.is_empty() {
        return Err(String::from("branch name cannot be empty"));
    }
    run_git_at(&repo_path, &["branch", "-D", branch_name])
}

#[tauri::command]
pub fn git_rename_branch(path: Option<String>, old_name: String, new_name: String) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let from = old_name.trim();
    let to = new_name.trim();
    if from.is_empty() || to.is_empty() {
        return Err(String::from("branch rename requires old and new names"));
    }
    run_git_at(&repo_path, &["branch", "-m", from, to])
}

#[tauri::command]
pub fn git_rename_remote_branch(
    path: Option<String>,
    remote_name: String,
    old_branch: String,
    new_name: String,
) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let remote = remote_name.trim();
    let from = old_branch.trim();
    let to = new_name.trim();
    if remote.is_empty() || from.is_empty() || to.is_empty() {
        return Err(String::from("remote branch rename requires remote, old branch, and new name"));
    }
    run_git_at(&repo_path, &["push", remote, &format!("{from}:{to}")])?;
    run_git_at(&repo_path, &["push", remote, "--delete", from])
}

#[tauri::command]
pub fn git_delete_remote_branch(path: Option<String>, remote_name: String, branch_name: String) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let remote = remote_name.trim();
    let branch = branch_name.trim();
    if remote.is_empty() || branch.is_empty() {
        return Err(String::from("remote branch delete requires remote and branch"));
    }
    run_git_at(&repo_path, &["push", remote, "--delete", branch])
}

#[tauri::command]
pub fn git_merge_branch(path: Option<String>, name: String) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let branch_name = name.trim();
    if branch_name.is_empty() {
        return Err(String::from("branch name cannot be empty"));
    }
    run_git_at(&repo_path, &["merge", branch_name])
}

#[tauri::command]
pub fn git_set_branch_tracking(path: Option<String>, branch_name: String, upstream: String) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let branch = branch_name.trim();
    let upstream_ref = upstream.trim();
    if branch.is_empty() || upstream_ref.is_empty() {
        return Err(String::from("tracking requires local and remote branch"));
    }
    run_git_at(
        &repo_path,
        &["branch", &format!("--set-upstream-to={upstream_ref}"), branch],
    )
}

#[tauri::command]
pub fn git_unset_branch_tracking(path: Option<String>, branch_name: String) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let branch = branch_name.trim();
    if branch.is_empty() {
        return Err(String::from("branch name cannot be empty"));
    }
    run_git_at(&repo_path, &["branch", "--unset-upstream", branch])
}

#[tauri::command]
pub fn git_blame_file(path: Option<String>, file_path: String) -> Result<Vec<GitBlameLineView>, String> {
    let client = open_git_client(path)?;
    let relative_path = file_path.trim();
    if relative_path.is_empty() {
        return Err(String::from("file path cannot be empty"));
    }
    let lines: Vec<BlameLine> = client.blame(relative_path).map_err(|error| error.to_string())?;
    Ok(lines
        .into_iter()
        .map(|line| GitBlameLineView {
            line_number: line.line_number,
            hash: line.hash,
            short_hash: line.short_hash,
            author: line.author,
            timestamp: line.timestamp,
            date: format_blame_date(line.timestamp),
            content: line.content,
        })
        .collect())
}

#[tauri::command]
pub fn git_tags_list(path: Option<String>) -> Result<Vec<GitTagView>, String> {
    let client = open_git_client(path)?;
    client
        .tag_list()
        .map(|tags| {
            tags.into_iter()
                .map(|tag| GitTagView {
                    name: tag.name,
                    hash: tag.hash,
                    timestamp: tag.timestamp,
                    message: tag.message,
                })
                .collect()
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn git_create_tag(path: Option<String>, name: String, message: String) -> Result<String, String> {
    git_create_tag_at(path, name, None, message)
}

#[tauri::command]
pub fn git_create_tag_at(
    path: Option<String>,
    name: String,
    target: Option<String>,
    message: String,
) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let tag_name = name.trim();
    if tag_name.is_empty() {
        return Err(String::from("tag name cannot be empty"));
    }
    let target_ref = target.unwrap_or_default();
    let annotation = message.trim();
    if annotation.is_empty() {
        if target_ref.trim().is_empty() {
            run_git_at(&repo_path, &["tag", tag_name])
        } else {
            run_git_at(&repo_path, &["tag", tag_name, target_ref.trim()])
        }
    } else if target_ref.trim().is_empty() {
        run_git_at(&repo_path, &["tag", "-a", tag_name, "-m", annotation])
    } else {
        run_git_at(&repo_path, &["tag", "-a", tag_name, target_ref.trim(), "-m", annotation])
    }
}

#[tauri::command]
pub fn git_delete_tag(path: Option<String>, name: String) -> Result<String, String> {
    let client = open_git_client(path)?;
    client.tag_delete(name.trim()).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn git_push_tag(path: Option<String>, name: String) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let tag_name = name.trim();
    if tag_name.is_empty() {
        return Err(String::from("tag name cannot be empty"));
    }
    match run_git_at(&repo_path, &["push", "origin", tag_name]) {
        Ok(output) => Ok(output),
        Err(_) => run_git_at(&repo_path, &["push", "--tags"]),
    }
}

#[tauri::command]
pub fn git_push_all_tags(path: Option<String>) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    run_git_at(&repo_path, &["push", "--tags"])
}

#[tauri::command]
pub fn git_remotes_list(path: Option<String>) -> Result<Vec<GitRemoteView>, String> {
    let client = open_git_client(path)?;
    client
        .remote_list()
        .map(|remotes| {
            remotes
                .into_iter()
                .map(|remote| GitRemoteView {
                    name: remote.name,
                    fetch_url: remote.fetch_url,
                    push_url: remote.push_url,
                })
                .collect()
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn git_add_remote(path: Option<String>, name: String, url: String) -> Result<String, String> {
    let client = open_git_client(path)?;
    client
        .remote_add(name.trim(), url.trim())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn git_set_remote_url(path: Option<String>, name: String, url: String) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let remote_name = name.trim();
    let remote_url = url.trim();
    if remote_name.is_empty() || remote_url.is_empty() {
        return Err(String::from("remote name and url are required"));
    }
    let output = run_git_at(&repo_path, &["remote", "set-url", remote_name, remote_url])?;
    if output.trim().is_empty() {
        Ok(format!("Updated remote '{}'.", remote_name))
    } else {
        Ok(output)
    }
}

#[tauri::command]
pub fn git_remove_remote(path: Option<String>, name: String) -> Result<String, String> {
    let client = open_git_client(path)?;
    client.remote_remove(name.trim()).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn git_fetch_remote(path: Option<String>, name: Option<String>) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let remote_name = name.unwrap_or_default();
    if remote_name.trim().is_empty() {
        let output = run_git_at(&repo_path, &["fetch"])?;
        Ok(if output.trim().is_empty() {
            String::from("Fetched all remotes.")
        } else {
            output
        })
    } else {
        let output = run_git_at(&repo_path, &["fetch", remote_name.trim()])?;
        Ok(if output.trim().is_empty() {
            format!("Fetched remote '{}'.", remote_name.trim())
        } else {
            output
        })
    }
}

#[tauri::command]
pub fn git_config_list(path: Option<String>) -> Result<Vec<GitConfigEntryView>, String> {
    let client = open_git_client(path)?;
    client
        .config_list()
        .map(|entries| {
            entries
                .into_iter()
                .map(|entry| GitConfigEntryView {
                    key: entry.key,
                    value: entry.value,
                    scope: entry.scope,
                })
                .collect()
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn git_set_config_value(path: Option<String>, key: String, value: String, global: bool) -> Result<String, String> {
    let client = open_git_client(path)?;
    client
        .config_set(key.trim(), &value, global)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn git_unset_config_value(path: Option<String>, key: String, global: bool) -> Result<String, String> {
    let client = open_git_client(path)?;
    client
        .config_unset(key.trim(), global)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn git_reset_to_commit(path: Option<String>, hash: String, mode: String) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let commit_hash = hash.trim();
    if commit_hash.is_empty() {
        return Err(String::from("commit hash cannot be empty"));
    }
    let reset_mode = match mode.trim() {
        "soft" => "soft",
        "hard" => "hard",
        _ => "mixed",
    };
    run_git_at(&repo_path, &["reset", &format!("--{reset_mode}"), commit_hash])
}

#[tauri::command]
pub fn git_amend_head_commit_message(path: Option<String>, hash: String, message: String) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let commit_hash = hash.trim();
    let new_message = message.trim();
    if commit_hash.is_empty() || new_message.is_empty() {
        return Err(String::from("commit hash and message are required"));
    }
    let head_hash = run_git_at(&repo_path, &["rev-parse", "HEAD"])?;
    if head_hash.trim() != commit_hash {
        return Err(String::from("Only the current HEAD commit can be amended."));
    }
    run_git_at(&repo_path, &["commit", "--amend", "-m", new_message])
}

#[tauri::command]
pub fn git_revert_commit(
    path: Option<String>,
    hash: String,
    no_commit: Option<bool>,
) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let commit_hash = hash.trim();
    if commit_hash.is_empty() {
        return Err(String::from("commit hash cannot be empty"));
    }
    let mut args: Vec<&str> = vec!["revert"];
    if no_commit.unwrap_or(false) {
        args.push("--no-commit");
    } else {
        // Skip the editor round-trip so the command returns without
        // blocking. git's default revert message carries the SHA and
        // is usually what the user wants.
        args.push("--no-edit");
    }
    args.push(commit_hash);
    run_git_at(&repo_path, &args)
}

#[tauri::command]
pub fn git_cherry_pick_commit(
    path: Option<String>,
    hash: String,
    no_commit: Option<bool>,
) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let commit_hash = hash.trim();
    if commit_hash.is_empty() {
        return Err(String::from("commit hash cannot be empty"));
    }
    let mut args: Vec<&str> = vec!["cherry-pick"];
    if no_commit.unwrap_or(false) {
        args.push("--no-commit");
    }
    args.push(commit_hash);
    run_git_at(&repo_path, &args)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitReflogEntry {
    pub hash: String,
    pub short_hash: String,
    pub ref_name: String,
    pub subject: String,
    pub relative_date: String,
}

#[tauri::command]
pub fn git_reflog_list(
    path: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<GitReflogEntry>, String> {
    let repo_path = repo_root(path)?;
    let count = limit.unwrap_or(100).clamp(1, 1000);
    // `\x1f` (ASCII unit separator) is the safest delimiter for
    // git log-style --format output: can't appear in refs, authors,
    // or subjects. Matches the pattern used in `log()` elsewhere.
    let sep = "\x1f";
    let format = format!("--format=%H{sep}%h{sep}%gD{sep}%gs{sep}%cr");
    let count_arg = format!("-{count}");
    let output = run_git_at(
        &repo_path,
        &["reflog", "--date=relative", count_arg.as_str(), format.as_str()],
    )?;
    let mut entries = Vec::new();
    for line in output.lines() {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(5, sep).collect();
        if parts.len() < 5 {
            continue;
        }
        entries.push(GitReflogEntry {
            hash: parts[0].to_string(),
            short_hash: parts[1].to_string(),
            ref_name: parts[2].to_string(),
            subject: parts[3].to_string(),
            relative_date: parts[4].to_string(),
        });
    }
    Ok(entries)
}

#[tauri::command]
pub fn git_drop_commit(path: Option<String>, hash: String, parent_hash: Option<String>) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let commit_hash = hash.trim();
    if commit_hash.is_empty() {
        return Err(String::from("commit hash cannot be empty"));
    }
    let head_hash = run_git_at(&repo_path, &["rev-parse", "HEAD"])?;
    if head_hash.trim() == commit_hash {
        return run_git_at(&repo_path, &["reset", "--hard", "HEAD~1"]);
    }
    let parent = parent_hash.unwrap_or_default();
    if parent.trim().is_empty() {
        return Err(String::from("parent hash is required to drop a non-HEAD commit"));
    }
    run_git_at(
        &repo_path,
        &["rebase", "--onto", parent.trim(), commit_hash, "HEAD"],
    )
}

#[tauri::command]
pub fn git_rebase_plan(path: Option<String>, count: Option<usize>) -> Result<GitRebasePlanView, String> {
    let repo_path = repo_root(path)?;
    let item_count = count.unwrap_or(10).clamp(1, 50);
    let rebase_merge_path = git_path_for_repo(&repo_path, "rebase-merge").ok();
    let rebase_apply_path = git_path_for_repo(&repo_path, "rebase-apply").ok();
    let in_progress = rebase_merge_path
        .as_ref()
        .is_some_and(|path| path.exists())
        || rebase_apply_path
            .as_ref()
            .is_some_and(|path| path.exists());

    let items = if in_progress {
        let todo_path = rebase_merge_path
            .as_ref()
            .map(|path| path.join("git-rebase-todo"))
            .filter(|path| path.exists())
            .or_else(|| {
                rebase_apply_path
                    .as_ref()
                    .map(|path| path.join("git-rebase-todo"))
                    .filter(|path| path.exists())
            });
        if let Some(todo_path) = todo_path {
            fs::read_to_string(todo_path)
                .unwrap_or_default()
                .lines()
                .filter_map(parse_rebase_line)
                .collect()
        } else {
            Vec::new()
        }
    } else {
        let output = run_git_at(
            &repo_path,
            &["log", "--format=%H%x1f%s", "-n", &item_count.to_string(), "HEAD"],
        )?;
        output
            .lines()
            .filter_map(|line| {
                let (hash, message) = line.split_once('\u{001f}')?;
                let hash = hash.trim().to_string();
                Some(GitRebaseItemView {
                    id: hash.clone(),
                    action: String::from("pick"),
                    short_hash: hash.chars().take(7).collect(),
                    hash,
                    message: message.trim().to_string(),
                })
            })
            .collect()
    };

    Ok(GitRebasePlanView { in_progress, items })
}

#[tauri::command]
pub fn git_execute_rebase(path: Option<String>, items: Vec<GitRebaseItemView>, onto: Option<String>) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    if items.is_empty() {
        return Err(String::from("rebase items cannot be empty"));
    }

    let base = onto
        .unwrap_or_default()
        .trim()
        .to_string();
    let resolved_base = if !base.is_empty() {
        base
    } else {
        let oldest = items.last().map(|item| item.hash.clone()).unwrap_or_default();
        if oldest.is_empty() {
            return Err(String::from("missing rebase base"));
        }
        format!("{oldest}~1")
    };

    let mut todo_text = String::new();
    for item in items.iter().rev() {
        if item.hash.trim().is_empty() {
            continue;
        }
        let action = if item.action.trim().is_empty() {
            "pick"
        } else {
            item.action.trim()
        };
        todo_text.push_str(&format!("{action} {} {}\n", item.hash.trim(), item.message.trim()));
    }

    let todo_path = unique_temp_path("pierx-rebase-todo", "txt");
    fs::write(&todo_path, todo_text)
        .map_err(|error| format!("Failed to write {}: {}", todo_path.display(), error))?;
    let script_path = create_sequence_editor_script(&todo_path)?;

    run_git_at_with_env(
        &repo_path,
        &["rebase", "-i", &resolved_base],
        &[("GIT_SEQUENCE_EDITOR", script_path.display().to_string())],
    )
}

#[tauri::command]
pub fn git_abort_rebase(path: Option<String>) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    run_git_at(&repo_path, &["rebase", "--abort"])
}

#[tauri::command]
pub fn git_continue_rebase(path: Option<String>) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    run_git_at(&repo_path, &["rebase", "--continue"])
}

#[tauri::command]
pub fn git_submodules_list(path: Option<String>) -> Result<Vec<GitSubmoduleView>, String> {
    let repo_path = repo_root(path)?;
    let urls_by_path = parse_gitmodules_urls(&repo_path);
    let output = run_git_at(&repo_path, &["submodule", "status", "--recursive"])?;
    Ok(output
        .lines()
        .filter_map(|line| {
            if line.trim().is_empty() {
                return None;
            }
            let status_symbol = line.chars().next()?.to_string();
            let rest = line.get(1..)?.trim();
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() < 2 {
                return None;
            }
            let hash = parts[0].trim().to_string();
            let path = parts[1].trim().to_string();
            let status = match status_symbol.as_str() {
                "-" => "uninitialized",
                "+" => "modified",
                "U" => "conflict",
                _ => "ok",
            };
            Some(GitSubmoduleView {
                path: path.clone(),
                commit_hash: hash.clone(),
                short_hash: hash.chars().take(7).collect(),
                status: status.to_string(),
                status_symbol,
                url: urls_by_path.get(&path).cloned().unwrap_or_default(),
            })
        })
        .collect())
}

#[tauri::command]
pub fn git_init_submodules(path: Option<String>) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    run_git_at(&repo_path, &["submodule", "init"])
}

#[tauri::command]
pub fn git_update_submodules(path: Option<String>, recursive: Option<bool>) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    if recursive.unwrap_or(true) {
        run_git_at(&repo_path, &["submodule", "update", "--init", "--recursive"])
    } else {
        run_git_at(&repo_path, &["submodule", "update", "--init"])
    }
}

#[tauri::command]
pub fn git_sync_submodules(path: Option<String>) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    run_git_at(&repo_path, &["submodule", "sync", "--recursive"])
}

#[tauri::command]
pub fn git_conflicts_list(path: Option<String>) -> Result<Vec<GitConflictFileView>, String> {
    let repo_path = repo_root(path)?;
    let output = run_git_at(&repo_path, &["diff", "--name-only", "--diff-filter=U"])?;
    let mut files = Vec::new();

    for entry in output.lines().map(str::trim).filter(|entry| !entry.is_empty()) {
        let absolute_path = repo_path.join(entry);
        let Ok(content) = fs::read_to_string(&absolute_path) else {
            continue;
        };
        let hunks = parse_conflict_hunks(&content);
        if hunks.is_empty() {
            continue;
        }
        let name = Path::new(entry)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(entry)
            .to_string();
        files.push(GitConflictFileView {
            name,
            path: entry.to_string(),
            conflict_count: hunks.len(),
            conflicts: hunks,
        });
    }

    Ok(files)
}

#[tauri::command]
pub fn git_conflict_accept_all(path: Option<String>, file_path: String, resolution: String) -> Result<String, String> {
    let repo_path = repo_root(path)?;
    let relative_path = file_path.trim();
    let resolved = match resolution.trim() {
        "theirs" => "theirs",
        _ => "ours",
    };
    if relative_path.is_empty() {
        return Err(String::from("file path cannot be empty"));
    }
    let content = fs::read_to_string(repo_path.join(relative_path))
        .map_err(|error| format!("Failed to read {}: {}", relative_path, error))?;
    let mut hunks = parse_conflict_hunks(&content);
    for hunk in &mut hunks {
        hunk.resolution = resolved.to_string();
    }
    write_resolved_conflict_file(&repo_path, relative_path, &hunks, resolved)?;
    Ok(format!("Accepted {} for {}", resolved, relative_path))
}

#[tauri::command]
pub fn git_conflict_mark_resolved(params: GitConflictMarkResolvedParams) -> Result<String, String> {
    let repo_path = repo_root(params.path)?;
    let relative_path = params.file_path.trim();
    if relative_path.is_empty() {
        return Err(String::from("file path cannot be empty"));
    }
    write_resolved_conflict_file(&repo_path, relative_path, &params.hunks, "ours")?;
    run_git_at(&repo_path, &["add", relative_path])?;
    Ok(format!("Marked {} as resolved", relative_path))
}
