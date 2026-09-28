# GPUI Base 换行补丁

## 原因与选择

原生 Windows release 的 360,022 字节超长单段测试中，`👩‍💻` 在软换行边界被拆成两行。进一步实测发现 `Shift+Left` 可能只选中 `é` 的组合附加符号。GPUI 的宽度估计与 Base 的光标边界使用 Unicode 标量，显示行和交互端点需要限定到完整字素。

公开接口允许开启 / 关闭软换行，但没有替换换行与光标边界策略的钩子。关闭软换行不满足需求，独立重写编辑器成本过高。因此维护一个范围受控的 Base 补丁。主 GPUI 仍统一为 gpui-pre 0.3.7，未引入第二套类型或 Zed editor。

## 来源与许可

- vendored crate：gpui-base 0.7.0，原始 crates.io 发布包。
- 上游提交：`0c830f4d257e69fdd17200650533ab4ca9a40cc0`，目录 `crates/base`。
- Apache-2.0；随源码保留 `vendor/gpui-base/LICENSE-APACHE`。
- Cargo `[patch.crates-io]` 让 Component、Kit 与应用统一使用这一份 Base。

## 改动范围

1. `src/input/editor/display_map/grapheme_wrap.rs`：把估算换行点向前收敛到字素起点，去重并排除空显示行。单个字素宽于行宽时允许该字素超宽，不拆分字素。
2. `text_wrapper.rs`：在原换行算法输出进入 WrapMap 前应用上述边界规则。
3. `display_map/mod.rs`：注册内部模块。
4. `src/input/base/grapheme_cursor.rs`：通过 Rope 块和 GraphemeCursor 寻找完整字素端点，正常路径不展平整篇文本。
5. `base/state.rs`：在可见光标 / 选区边界规范化处调用该策略；新增 `reveal_cursor`，复用组件滚动逻辑，在视口重排后显示当前光标，保留选区方向与撤销历史。
6. `input/mod.rs`：注册光标边界模块。

文档文本、撤销算法、精确 UTF-8 / UTF-16 源码接口和保存逻辑不变。修改的是显示行分界和可见光标 / 选区端点。

原锁定的 unicode-segmentation 1.12.0 在跨 Rope 块 ZWJ 测试中也返回了错误边界。1.13.3 发布源码包含相应 chunk-boundary 修复测试，依赖已统一锁到 1.13.3；同一回归在升级后通过。参见[上游源码](https://docs.rs/crate/unicode-segmentation/1.13.3/source/src/grapheme.rs)。

## 维护与验证

- `cargo test --test grapheme_wrap` 编译实际补丁文件，覆盖 ZWJ emoji、组合附加符号、旗帜和超宽单个字素。
- `cargo test --test grapheme_cursor` 跨多个 Rope 块，对每个标量位置比较完整字符串字素边界；覆盖 ZWJ、旗帜序列与长组合字符序列。
- GPUI 键盘事件测试实际执行 Shift+Left、删除和撤销，确认不会拆开 emoji / 组合字符。
- `cargo test --locked` 同时运行应用和接口回归。
- 必须复测原生软换行、鼠标选择、中文候选框和窗口重排；单元测试不能代替。
- 升级 Base 时比较这六个文件，确认新上游是否已处理字素边界；若已修复则移除 patch 和 vendor。
- vendor 包约 4 MB；维护重点是显示行与源码偏移一致，不能只检查渲染结果。

当前原生回归结果以 TESTING.md 为准。

## 视口重排

实测最大化后还原窗口会保留选区，却把选区留在可见区域之外。组件公开的定位方法会改变选区，因此新增上述窄接口。应用观察编辑区域尺寸，只在原光标可见且编辑器有焦点时，于下一帧调用它；用户主动滚离光标时不会强制滚回。可见性判断把未滚动的光标几何加上滚动偏移，再与输入区域相交。

自动回归覆盖变宽再变窄、反链栏出现导致编辑区变矮，验证源文、选区方向、光标可见性。最终版本已原生复查：360,022 字节单段的文末选区在最大化、还原后仍可见，源码范围与文件哈希不变。升级 Base 时还需核对布局通知时机和 `scroll_to` 行为。
