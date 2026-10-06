# Inkstone patches to gpui-component 0.7.0

The crate is copied from the locked crates.io release, with its Apache license retained.

`src/input/popovers/completion_menu.rs` adjusts completion suggestions to the installed Obsidian reference: 15px text, 1.4 line height, 6px vertical and 12px horizontal padding, 4px corners, 6px menu padding, and a 300px maximum height. The host sets the existing width option to 500px.

Completion highlighting no longer interprets `CompletionItem.filter_text.len()` as a byte range in the visible label. Filter text may contain aliases or paths that differ from the label. A literal matching prefix is bold; an empty or unrelated query leaves the text unchanged.

The menu renders `label_details.description` below the label at 12px. Inkstone supplies descriptions for every row so the list retains a uniform row height. Optional `data.inkstone_match_ranges` carries matches computed against the visible label; the renderer checks UTF-8 boundaries before making those ranges bold.

Completion placement uses window coordinates with an 8px edge margin. It chooses the space above the caret when the preferred list height will not fit below and limits the list height to the available space. The list keeps its existing scroll and keyboard-selection behavior.

`src/title_bar.rs` allows the title-bar content region to shrink below its intrinsic width. The window controls remain non-shrinking, so a host's scrollable tabs cannot push minimize/maximize/close outside the window.

`src/switch.rs` gives `Size::Large` a 40×22 track and an 18px thumb with the existing 2px inset, matching the reference modal controls. Small and medium sizes retain their existing dimensions. Inkstone selects the large size for its settings and property dialogs.

`src/menu/popup_menu.rs` forwards the actual mouse `ClickEvent` to both standard and custom item callbacks, preserving modifiers and position. Keyboard confirmation retains its existing default event. History-menu Ctrl-click depends on this information to navigate in a copied view. Menu item elements expose stable debug selectors for the mouse-event regression.

`src/menu/context_menu.rs` adds an opt-in `long_press(Duration)` gesture. The host uses 400ms for navigation controls. Releasing early cancels the timer; movement beyond 5px cancels it, with predominantly downward movement opening the menu immediately. Held menus appear below the trigger and consume the initiating release, optionally confirming the hovered menu item with the real mouse modifiers. Existing right-click menus keep their default behavior. Timer callbacks use weak state, and the menu's trigger position is tracked separately from its popup position so shared element state still draws one menu.

`src/slider.rs` exposes an accessible-name builder and forwards it to the base slider, and reserves a transparent border whose color changes on keyboard focus, without resizing on focus changes. A box shadow behind the transparent track filled its interior on macOS, so the border replaces that shadow. The matching `gpui-base/src/slider.rs` patch makes enabled single-value sliders Tab stops and handles unmodified arrows, Home/End and PageUp/PageDown. Discrete keyboard and accessibility adjustments emit Change/Release only when the value changes, clamp to bounds, and preserve the start of a range when adjusting its end. Disabled sliders do not register increment/decrement handlers. Range-thumb keyboard navigation is unchanged; Inkstone uses single-value sliders. Programmatic set_value remains silent. Upgrade checks: both crates' slider tests and Inkstone's settings_sliders_accept_keyboard_changes_and_keep_focus regression.
