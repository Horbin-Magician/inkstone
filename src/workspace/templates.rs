use super::*;

impl Workspace {
    pub(super) fn insert_current_date_time(
        &mut self,
        time: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ui.name_mode.is_some()
            || self.ui.quick_open
            || self.ui.settings
            || self.ui.property_open
        {
            return;
        }
        let Some(pane) = self.current_pane() else {
            return;
        };
        if pane.read(cx).reading {
            return;
        }
        let text = match self.ui.prefs.templates.expand(
            if time { "{{time}}" } else { "{{date}}" },
            "",
            &chrono::Local::now(),
        ) {
            Ok(text) => text,
            Err(error) => {
                self.status = error;
                cx.notify();
                return;
            }
        };
        pane.update(cx, |pane, cx| {
            pane.editor.update(cx, |editor, cx| {
                let cursor = editor.cursor();
                let after = cursor + text.len();
                editor.apply_source_edit(cursor..cursor, &text, after..after, window, cx);
                editor.focus(window, cx);
            });
        });
    }
    pub(super) fn prepare_template_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = &self.ui.prefs.templates;
        for (input, value) in self.ui.template_inputs.iter().zip([
            &settings.folder,
            &settings.date_format,
            &settings.time_format,
        ]) {
            input.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
        }
    }
    pub(super) fn template_settings_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let now = chrono::Local::now();
        let preview = self
            .ui
            .prefs
            .templates
            .expand("{{date}} {{time}}", "", &now)
            .unwrap_or_else(|error| error);
        div()
            .flex()
            .gap_4()
            .flex_1()
            .min_h_0()
            .child(self.settings_nav(cx))
            .child(
                div()
                    .id("settings-content")
                    .track_scroll(&self.ui.settings_scroll)
                    .relative()
                    .vertical_scrollbar(&self.ui.settings_scroll)
                    .overflow_y_scroll()
                    .min_h_0()
                    .h_full()
                    .flex_1()
                    .min_w_0()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .children(
                        [
                            (
                                "模板文件夹位置",
                                "指定库内文件夹，插入模板时包含其中的子文件夹。",
                            ),
                            ("日期格式", "用于 {{date}}；留空使用 YYYY-MM-DD。"),
                            ("时间格式", "用于 {{time}}；留空使用 HH:mm。"),
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(i, (title, description))| {
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_4()
                                .pb_4()
                                .child(
                                    div().flex_1().min_w_0().child(title).child(
                                        div()
                                            .text_size(px(13.))
                                            .line_height(relative(1.4))
                                            .whitespace_normal()
                                            .text_color(rgb(0x999999))
                                            .child(description),
                                    ),
                                )
                                .child(
                                    div()
                                        .w(px(220.))
                                        .flex_shrink_0()
                                        .child(Input::new(&self.ui.template_inputs[i])),
                                )
                        }),
                    )
                    .child(div().text_sm().child(format!("当前预览：{preview}")))
                    .when_some(self.ui.prefs.templates.directory().err(), |s, error| {
                        s.child(div().text_sm().child(error))
                    }),
            )
            .into_any_element()
    }
    pub(super) fn open_template_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let folder = self.ui.prefs.templates.directory().and_then(|folder| {
            let vault = self
                .vault
                .as_ref()
                .ok_or_else(|| "请先打开笔记库。".to_string())?;
            let absolute = vault
                .folder_path(&folder)
                .map_err(|error| error.to_string())?;
            if !absolute.is_dir() {
                return Err(format!("模板文件夹不存在：{}", folder.display()));
            }
            Ok(folder)
        });
        if let Err(error) = folder {
            self.status = error;
            cx.notify();
            return;
        }
        self.focus_search(false, window, cx);
        self.ui.template_mode = true;
        self.search_results.clear();
        self.run_search(cx);
        cx.notify();
    }
}
