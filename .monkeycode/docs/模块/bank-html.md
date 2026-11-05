# bank-html

解析与序列化《题库格式化规范》HTML。

- `parse.rs`：读 `h1`/`h2`/`h3`/`.question`/`.options`/`.answer`
- 题型：选择按答案字母数分单选/多选；判断/填空/编程按标题关键词
- `serialize.rs`：UTF-8、`lang="zh-CN"`、黑白样式、系统字体
- 往返要求语义字段相等，不要求字节级空白相同

夹具在 `testdata/bank-html/`。
