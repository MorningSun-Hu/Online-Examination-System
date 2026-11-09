# 开发者指南

## 项目目的

教室局域网考试：教师机提供讲台与学生页。本仓库是完整应用，无外部账号体系。

**核心职责**

- 解析/维护题库并组可复用试卷
- 发布考试或练习，学生匹配进入并续答
- 单选/多选/判断自动批改，填空可开关，编程始终人工给分，公开后回发分析

## 环境搭建

### 前置条件

- Rust（cargo）
- Node.js（前端 `npm run build` / `npm run dev`）

### 启动

教室演示（构建前端后跑后端）：

```bash
# 构建前端并启动 exam-server
./scripts/start-teacher.sh run
```

开发（后端 + Vite 热更新）：

```bash
./scripts/start-teacher.sh dev
```

只构建前端：

```bash
./scripts/start-teacher.sh build
```

Windows 用 `scripts/start-teacher.bat`（CRLF，`goto` 分支）。

手动等价：

```bash
cd web
npm install
npm run build
cd ..
EXAM_WEB_DIR=./web/dist cargo run -p exam-server
```

默认绑定 `0.0.0.0:8080`，库文件 `./data/exam.db`。

### 环境变量

| 变量 | 默认 | 说明 |
|---|---|---|
| `EXAM_DATA_DIR` | `./data` | 数据目录，库文件为其中 `exam.db` |
| `EXAM_BIND` | `0.0.0.0:8080` | 监听地址 |
| `EXAM_WEB_DIR` | 探测 `web/dist` 等 | 静态资源目录 |

测试用 `Config::from_data_dir` 指向临时目录，不读本机 `./data`。

## 验证

```bash
cargo test -p bank-html --offline
cargo test -p paper-gen --offline
cargo test -p grading --offline
cargo test -p exam-server --offline
cd web && npm run build
```

`cargo test` 不更新运行中的二进制；改完入口后需重新 `cargo run`。

冒烟：`GET /api/health` 返回 `ok`；浏览器打开 `/teacher.html` 设口令后进入讲台。

## 目录约定

- 需求与设计在 `.monkeycode/specs/2026-11-01-online-exam-system/`
- 项目 wiki 在 `.monkeycode/docs/`
- 不提交 `target/`、`web/dist/`、`web/node_modules/`、`data/`、真实口令

## 协作

- 提交身份：`git -c user.name='huchenyang' -c user.email='hcy_1987@163.com'`
- 提交须用户明确授权；「提交」不含推送
- 删除文件前列完整路径并等确认
