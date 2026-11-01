# 通用开发记忆（可复制到其他项目）

把本文件复制到目标仓库的 `.monkeycode/MEMORY.md`（或与该文件合并）。只记录「以后每次怎么做」，不记录某个功能做了什么。

## 格式

### User Instruction Entry

[条目标题]
- Date: [YYYY-MM-DD]
- Context: [场景]
- Instructions:
  - [具体做法]

---

## Entries

[协作启动]
- Date: 2026-09-19
- Context: 从 CampusLink-NCMS 协作中抽出，适用于后续所有项目
- Instructions:
  - 每次新对话、上下文压缩或任务阶段切换时，先读取项目 `.monkeycode/MEMORY.md`
  - 只做用户当前明确要求的范围，不擅自扩大改动
  - 回复与思考过程使用简体中文
  - 发现可复用的操作/构建/排障/协作约定时，写入 MEMORY；实现细节留在代码和需求文档里

[Git 身份与提交授权]
- Date: 2026-09-19
- Context: 用户要求所有仓库统一身份，且提交必须先授权
- Instructions:
  - 提交身份固定为 `huchenyang <hcy_1987@163.com>`
  - 用 `git -c user.name='huchenyang' -c user.email='hcy_1987@163.com'` 覆盖当次提交，不修改仓库 `git config`
  - 提交前必须取得用户明确授权（例如「提交」「授权提交」）
  - 推送必须另有明确授权（例如「推送」）；「提交」不包含推送
  - 提交前检查 `git status`、`git diff`、`git log`，只暂存本次相关文件
  - 不提交密钥、真实 API Key、`.env` 真值、以及 `target/`、`dist/`、`node_modules/` 等可重建产物
  - 不 amend、不 force push，除非用户明确要求
  - 提交说明使用约定式前缀：`feat` / `fix` / `chore` / `docs` / `refactor`

[删除规定]
- Date: 2026-09-19
- Context: 清理构建缓存与过时静态资源时形成的安全删除流程
- Instructions:
  - 删除前必须列出完整路径，并核对：是否 gitignore、是否被当前入口文件引用、删除后能否重建
  - 必须等待用户明确确认后再删除
  - 只删除确认清单内的项，不顺带清理未列出的文件
  - 已跟踪文件删除会使工作区变脏，要入库时必须再次取得提交授权
  - 磁盘不足时优先列出可重建缓存（编译 `target`、前端 `dist`、旧哈希静态文件），确认后再清

[文档驱动开发]
- Date: 2026-09-19
- Context: 文档化开发阶段的通用流程
- Instructions:
  - 编码前先读需求文档、任务拆解、模块路线图
  - 每个开发阶段开始前先写执行基线文档，编码、联调、验收以该基线为准
  - 用户要求先做需求分析、目录规划时，先出文档，再开始业务编码

[构建与交叉编译]
- Date: 2026-09-19
- Context: 小磁盘环境与多平台编译中的通用做法
- Instructions:
  - 长编译、打包用受管后台终端，设置合理超时与 CPU 限制，避免把磁盘写满
  - 交叉编译用环境变量指定 linker（例如 `CARGO_TARGET_*_LINKER`），不在仓库写死会污染其他平台构建的 `.cargo/config.toml`
  - Windows 批处理使用 CRLF；避免 `if (...) else (...)` 块结构，改用 `goto`
  - `cargo test` 不会更新运行用二进制；改完入口后需先 `cargo build` 再启动验证

[预览与本地验证]
- Date: 2026-09-19
- Context: 预览服务与冒烟测试约定
- Instructions:
  - 用户要求停止预览或服务时立即停止对应进程
  - 冒烟测试使用独立数据文件或临时目录，避免覆盖用户本机数据
  - 前后端分离时，预览入口走前端开发服务器，并把 `/api` 反代到后端

[密钥隔离]
- Date: 2026-09-19
- Context: Agent 环境与用户项目凭据必须分开
- Instructions:
  - 不扫描、不读取、不输出、不写入执行环境中的大模型 API Key
  - 用户项目使用项目自己的环境变量名和占位符，由用户自行填入真实值

[apply_patch 标记格式]
- Date: 2026-11-01
- Context: 合并 Begin/End 约定，以及改 Markdown 列表时大量 Failed to find context 的教训
- Instructions:
  - apply_patch 的 `patchText` 必须以整行 `*** Begin Patch` 开头、整行 `*** End Patch` 结尾
  - 这两行必须精确匹配：行首无空格、行尾无多余 `***`、无引号、无代码围栏包裹
  - 错误写法 `*** Begin Patch ***` / `*** End Patch ***` 会报 `Invalid patch format: missing Begin/End markers`
  - 不要写 unified diff 行号头（例如 `@@ -64,6 +64,19 @@`）；也不要写 `@@ - **列表项**`，`@@ -` 会被当成 unified diff
  - `@@` 后面必须是文件里真实存在的整行，不要用子串。`Failed to find context` 就是定位行与文件对不上
  - `@@` 不要选以 `- ` 开头的 Markdown 列表项；优先用标题，例如 `# 需求实施计划`、`## Glossary`、`### Requirement 8`
  - 不要把列表行当作未标记上下文（会被当成删除）。标题 `@@` 定位后直接写变更的 `+`/`-` 行
  - 变更「以 `- ` 开头」的列表行：删除写 `-- 原文本`（第一个 `-` 是操作符，后面的 `- ` 才是文件内容）；新增写 `+- 新文本`
  - 带缩进的列表行（如 `  - [ ] 2.4 ...`）：删除写 `-  - [ ] 2.4 ...`；新增写 `+  - [ ] 2.4 ...`
  - 不相邻的多处修改不要放进同一个 hunk。一次补丁只改一处列表行，成功后再改下一处
  - 同一补丁里不要重复多次相同的 `@@ 标题` 去改多处，会报 Failed to find context
  - 新建文件用 `*** Add File: <path>`，后续每行内容必须以 `+` 开头
  - 调用 apply_patch 前按本条自检：Begin/End、`@@` 定位行、列表破折号转义
