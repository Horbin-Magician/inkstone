# Inkstone patches to gpui-component 0.7.0

The crate is copied from the locked crates.io release, with its Apache license retained.

`src/input/popovers/completion_menu.rs` adjusts completion suggestions to the installed Obsidian reference: 15px text, 1.4 line height, 6px vertical and 12px horizontal padding, 4px corners, 6px menu padding, and a 300px maximum height. The host sets the existing width option to 500px.

Completion highlighting no longer interprets `CompletionItem.filter_text.len()` as a byte range in the visible label. Filter text may contain aliases or paths that differ from the label. A literal matching prefix is bold; an empty or unrelated query leaves the text unchanged.

The menu renders `label_details.description` below the label at 12px. Inkstone supplies descriptions for every row so the list retains a uniform row height. Optional `data.inkstone_match_ranges` carries matches computed against the visible label; the renderer checks UTF-8 boundaries before making those ranges bold.

Completion placement uses window coordinates with an 8px edge margin. It chooses the space above the caret when the preferred list height will not fit below and limits the list height to the available space. The list keeps its existing scroll and keyboard-selection behavior.

`src/title_bar.rs` allows the title-bar content region to shrink below its intrinsic width. The window controls remain non-shrinking, so a host's scrollable tabs cannot push minimize/maximize/close outside the window.

`src/switch.rs` gives `Size::Large` a 40×22 track and an 18px thumb with the existing 2px inset, matching the reference modal controls. Small and medium sizes retain their existing dimensions. Inkstone selects the large size for its settings and property dialogs.

`src/menu/popup_menu.rs` forwards the actual mouse `ClickEvent` to both standard and custom item callbacks, preserving modifiers and position. Keyboard confirmation retains its existing default event. History-menu Ctrl-click depends on this information to navigate in a copied view. Menu item elements expose stable debug selectors for the mouse-event regression.
