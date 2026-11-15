# 工作约定

- 按功能或可独立验证的改动分步提交。
- 按改动范围完成必要检查后及时提交，不积累多轮改动再集中提交。
- 不将用户已有笔记修改混入代码提交。

## 提交信息

- 使用英文撰写标题和正文。标题遵循 Conventional Commits，格式为 `type(scope): description`。省略 scope 时使用 `type: description`。
- type 使用小写，按改动意图选择 `feat`、`fix`、`docs`、`refactor`、`perf`、`test`、`style`、`build`、`ci`、`chore` 或 `revert`。scope 可填写受影响的模块，如 `vault` 或 `editor`。
- description 以小写英文动词原形开头，简明说明本次改动，末尾不加句号。需要解释原因或影响时，在标题后空一行再写正文。

示例：

```text
fix(vault): skip blank tabs during file refresh
docs: document commit message conventions
```
