# 软件管理 — 自定义条目（software-extras.json）

Pier-X 内置了 31 个常用软件的安装/探测/详情元数据。需要管理自己常用但
未内置的软件时，把条目写到 `software-extras.json` 即可：Pier-X 启动时会
合并到主目录里，就跟内置条目一样能在面板里搜索、安装、查看详情。

## 文件位置

软件管理面板底部会显示完整路径，点击即可复制到剪贴板。各平台默认位置：

| 系统 | 路径 |
|---|---|
| Windows | `%APPDATA%\com.pier-x\software-extras.json` |
| macOS | `~/Library/Application Support/com.pier-x/software-extras.json` |
| Linux | `~/.config/com.pier-x/software-extras.json` |

文件不存在或解析失败时，**面板照常工作**，只用内置目录；解析失败的具体
原因会写到日志（启动日志 `pier-x.log`）。

## 格式

支持两种顶层结构，**新写代码用 wrapper 形式**（可同时声明软件 + 组合）：

### Wrapper 形式（推荐）

```json
{
  "packages": [
    { "id": "redis-stack", "displayName": "Redis Stack", "probeCommand": "...", "installPackages": { "apt": ["redis-stack-server"] } }
  ],
  "bundles": [
    {
      "id": "my-stack",
      "displayName": "我的开发栈",
      "description": "团队常用三件套",
      "packageIds": ["docker", "git", "redis-stack"]
    }
  ]
}
```

### Bundle 字段

| 字段 | 必填 | 类型 | 说明 |
|---|---|---|---|
| `id` | ✓ | string | 全局唯一；与内置 bundle id（`devops` / `java-dev` / `container-ops` / `lamp` / `diagnostics`）冲突时**该 bundle 被忽略** |
| `displayName` | ✓ | string | 卡片显示名 |
| `description` | | string | 卡片副标题 |
| `packageIds` | ✓ | string[] | 引用的 packages id（**支持引用 extras 里你自己的 packages**），未知 id 在安装时静默跳过 |

### 旧的纯数组形式（向后兼容）

```json
[
  { "id": "...", "displayName": "...", "probeCommand": "...", "installPackages": {...} }
]
```

旧写法等价于 `{ "packages": [...] }`，bundles 留空。

### Package 字段

顶层是数组，每条是一个对象：

```json
[
  {
    "id": "redis-stack",
    "displayName": "Redis Stack",
    "category": "database",
    "binaryName": "redis-stack-server",
    "probeCommand": "command -v redis-stack-server >/dev/null 2>&1 && redis-stack-server --version 2>&1",
    "installPackages": {
      "apt": ["redis-stack-server"],
      "dnf": ["redis-stack-server"]
    },
    "serviceUnits": {
      "apt": "redis-stack-server",
      "dnf": "redis-stack-server"
    },
    "configPaths": ["/etc/redis-stack.conf"],
    "defaultPorts": [6379],
    "dataDirs": ["/var/lib/redis-stack"],
    "supportsReload": false,
    "notes": "Redis with all the modules: RedisJSON / Search / TimeSeries / Bloom / Graph"
  }
]
```

### 字段说明

| 字段 | 必填 | 类型 | 说明 |
|---|---|---|---|
| `id` | ✓ | string | 全局唯一标识，全小写，dash 分隔；与内置条目冲突时**条目会被忽略** |
| `displayName` | ✓ | string | 面板里显示的名字 |
| `probeCommand` | ✓ | string | 用来检测是否已安装 + 提取版本的 shell 命令；约定：`command -v <bin> >/dev/null 2>&1 && <bin> --version 2>&1` |
| `installPackages` | ✓ | object | key 是包管理器（`apt`/`dnf`/`yum`/`apk`/`pacman`/`zypper`），value 是非空数组 |
| `category` | | string | 用于分类分段（`database`/`web`/`runtime`/`dev`/`editor`/`terminal`/`network`/`text`/`system`/`container`），其它值会归到"其它" |
| `binaryName` | | string | 详情页里 `command -v` 的目标，比 id 更准确（例如 `psql` 对应 `postgres` 条目） |
| `serviceUnits` | | object | systemd 单元名映射，key 同 installPackages 的 key |
| `configPaths` | | string[] | 详情页展示的配置文件路径，会先 `test -e` 过滤 |
| `defaultPorts` | | number[] | 默认监听端口，详情页会和 `ss -ltn` 结果对比 |
| `dataDirs` | | string[] | 卸载对话框里"也清空数据目录"勾选项作用的目录 |
| `supportsReload` | | bool | 是否支持 `systemctl reload` 不中断重载（如 nginx），默认 `false` |
| `notes` | | string | 行内灰字提示 |

### 哪些字段不能在 extras 里设置

- `vendorScript` — 上游官方脚本走 `curl|sh`，需要审计 URL；目前只允许在
  内置条目里声明，避免 extras 文件被偷换成恶意脚本来源。
- `versionVariants` — Java 8/11/17/21 这类版本级联属于内置功能，不开放
  给 extras（设计需要 schema 演进，先不暴露）。
- `cleanup_scripts` — 同上游脚本一样属于内置安全边界。如果你的自定义
  软件确实需要写额外的源/repo 文件，建议封装一个上游脚本（手工跑），
  而不是通过 extras 自动化。

## 校验规则

启动时按顺序应用：

1. JSON 解析失败 → **整个文件忽略**，写日志 `WARN`。
2. 单条 entry 校验失败（缺 id / displayName / probeCommand，或 `installPackages` 为空，或
   引用了未知包管理器）→ **只忽略这条**，其它条目继续生效。
3. id 与内置条目冲突 → **忽略这条**。

## 改完不会自动生效

为了避免 panel 频繁重建注册表，extras 文件的修改**只在 Pier-X 重启后生效**。
改完后用 ⌘Q / 关掉 Pier-X 再开。

## 例子：私有 yum 仓库里的内部工具

```json
[
  {
    "id": "acme-cli",
    "displayName": "Acme CLI (内部工具)",
    "category": "system",
    "binaryName": "acme",
    "probeCommand": "command -v acme >/dev/null 2>&1 && acme --version 2>&1",
    "installPackages": {
      "dnf": ["acme-cli"],
      "yum": ["acme-cli"]
    },
    "configPaths": ["/etc/acme/acme.yaml"],
    "notes": "需要先在 /etc/yum.repos.d/ 配置内部仓库"
  }
]
```

## 例子：覆盖已经在公司镜像里有的小工具

```json
[
  {
    "id": "yq",
    "displayName": "yq",
    "category": "text",
    "binaryName": "yq",
    "probeCommand": "command -v yq >/dev/null 2>&1 && yq --version 2>&1",
    "installPackages": {
      "apt": ["yq"],
      "dnf": ["yq"],
      "apk": ["yq"]
    }
  }
]
```
