# paper-gen

按 `PaperRule` 抽题并分配分值。

- 题型分组顺序固定，组内可乱序
- 选择题可选项乱序，快照保存 `option_map`
- `allocate_scores`：同题型余数加在最后几题；`T < N` 报错
- 属性测试：分值总和、选项映射可逆
