use super::*;

impl Workspace {
    pub(super) fn prepare_daily_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = &self.ui.prefs.daily;
        for (input, value) in self.ui.daily_inputs.iter().zip([
            &settings.format,
            &settings.folder,
            &settings.template,
        ]) {
            input.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
        }
    }
    pub(super) fn daily_settings_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let now = chrono::Local::now();
        let preview = self
            .daily_path(&now)
            .map(|path| format!("当前预览：{}", path.display()))
            .unwrap_or_else(|error| error);
        div().flex().gap_4().min_h(px(330.)).child(self.settings_nav(cx)).child(
            div().flex_1().min_w_0().p_3().flex().flex_col().gap_5()
                .children([
                    ("日期格式", "支持 YYYY、MM、DD、HH、mm、ss；用 / 划分子文件夹，用 [文字] 保留文字。"),
                    ("新建日记的存放位置", "库内文件夹；留空沿用新建笔记的存放位置。"),
                    ("模板文件位置", "首次创建日记时插入；支持 {{date}}、{{time}}、{{title}} 和 {{date:YYYY-MM-DD}}。"),
                ].into_iter().enumerate().map(|(i, (title, description))| {
                    div().flex().items_center().justify_between().gap_4().pb_4()
                        .child(div().flex_1().min_w_0().child(title).child(div().text_size(px(13.)).line_height(relative(1.4)).whitespace_normal().text_color(rgb(0x999999)).child(description)))
                        .child(div().w(px(220.)).flex_shrink_0().child(Input::new(&self.ui.daily_inputs[i])))
                }))
                .child(div().text_sm().child(preview))
                .when_some(self.ui.prefs.daily.template_path().err(), |s, error| s.child(div().text_sm().child(error)))
        ).into_any_element()
    }
    fn daily_path(&self, now: &chrono::DateTime<chrono::Local>) -> Result<PathBuf, String> {
        let settings = &self.ui.prefs.daily;
        let current = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map(|tab| tab.path.as_path());
        let folder = if settings.folder.trim().is_empty() {
            self.ui
                .prefs
                .locations
                .directory(current, false)
                .map_err(str::to_owned)?
        } else {
            PathBuf::new()
        };
        settings.path(now, &folder)
    }
    pub(super) fn open_daily(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault.clone() else {
            self.choose_vault(window, cx);
            return;
        };
        if self.ui.file_operation || self.ui.pending_file_writes > 0 {
            self.status = "请等待当前文件操作完成。".into();
            cx.notify();
            return;
        }
        let now = chrono::Local::now();
        let settings = self.ui.prefs.daily.clone();
        let path = self.daily_path(&now);
        let path = match path {
            Ok(path) => path,
            Err(error) => {
                self.status = error;
                cx.notify();
                return;
            }
        };
        if let Some(tab) = self.tabs.iter().find(|t| {
            t.path
                .to_string_lossy()
                .eq_ignore_ascii_case(&path.to_string_lossy())
        }) {
            self.open_note(tab.path.clone(), window, cx);
            return;
        }
        let path = self
            .files
            .iter()
            .find(|p| {
                p.to_string_lossy()
                    .eq_ignore_ascii_case(&path.to_string_lossy())
            })
            .cloned()
            .unwrap_or(path);
        let generation = self.generation;
        self.ui.pending_file_writes += 1;
        let task = cx.background_executor().spawn(async move {
            let result = (|| -> Result<(), String> {
                if vault.read(&path).map_err(|e| e.to_string())?.is_some() {
                    return Ok(());
                }
                if !settings.folder.trim().is_empty() {
                    let folder = vault.root.join(settings.folder.trim().replace('\\', "/"));
                    if !folder.is_dir() {
                        return Err("日记文件夹不存在，请先创建该文件夹。".into());
                    }
                }
                let text = if let Some(template) = settings.template_path()? {
                    let source = vault
                        .read(&template)
                        .map_err(|e| e.to_string())?
                        .ok_or_else(|| format!("找不到日记模板：{}", template.display()))?;
                    let title = path.file_stem().unwrap_or_default().to_string_lossy();
                    inkstone::daily::expand_template(&source, &title, &now)?
                } else {
                    String::new()
                };
                vault.create(&path, &text).map_err(|e| e.to_string())?;
                Ok(())
            })();
            (path, result)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (path, result) = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(()) => {
                        this.rescan = true;
                        this.refresh_requested = true;
                        this.open_note(path, window, cx);
                    }
                    Err(error) => {
                        this.status = error;
                        this.ui.window_close_requested = false;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
