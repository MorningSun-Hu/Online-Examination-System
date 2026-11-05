# 执行基线（编码阶段）

Date: 2026-11-05

## 依据

- 需求：`.monkeycode/specs/2026-11-01-online-exam-system/requirements.md`
- 设计：`.monkeycode/specs/2026-11-01-online-exam-system/design.md`
- 任务：`.monkeycode/specs/2026-11-01-online-exam-system/tasklist.md`（测试任务全部必做）

## 本阶段做法

- 按 tasklist 顺序实施，编码、联调、验收以上述三份文档为准。
- 题库夹具：用户上传的 `Python 程序设计基础练习 200 题（含答案）.html` 放到 `testdata/bank-html/`；规范结构另备小夹具。
- 数据文件只用 `./data/` 或测试临时目录，不碰用户本机已有库。
- 提交须用户明确授权；不改仓库 git config。

## 当前状态（2026-11-05）

- 任务 1–21 已完成。项目 wiki 在 `.monkeycode/docs/`。
- 讲台：题库、班级名册、试卷、任务（当前/历史）、成绩数据大屏。
- 试卷与任务分离；分值在试卷预览调整；同题型余数加在最后几题。
- 学生公开后选择题列出选项，并标出选择与正确答案。
- 启动：`./scripts/start-teacher.sh run`；健康检查 `GET /api/health`。
