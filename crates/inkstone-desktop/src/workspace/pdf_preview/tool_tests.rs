use super::*;
use core::prelude::v1::test;

fn open(pdf: &Path, cx: &mut TestAppContext) -> (WindowHandle<Reader>, VisualTestContext) {
    let handle = cx.add_window(|w, cx| Reader::new(pdf.to_owned(), w, cx));
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(900.), px(700.)));
    for _ in 0..300 {
        visual.executor().advance_clock(Duration::from_millis(15));
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        if visual.debug_bounds("pdf-page-0").is_some() {
            return (handle, visual);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("first PDF page did not render");
}
fn click(v: &mut VisualTestContext, id: &'static str) {
    v.update(|w, cx| w.draw(cx).clear(cx));
    let pos = v
        .debug_bounds(id)
        .unwrap_or_else(|| panic!("missing {id}"))
        .center();
    v.simulate_click(pos, Modifiers::default());
}
fn fixture(dir: &Path) -> PathBuf {
    let pdf = dir.join("paper.pdf");
    std::fs::write(
        &pdf,
        include_bytes!("../../../tests/fixtures/two-pages.pdf"),
    )
    .unwrap();
    pdf
}
#[gpui::test]
#[ignore = "requires packaged PDFium beside the test executable"]
fn pdf_page_text_edits_and_reopens(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    cx.update(init);
    let dir = tempfile::tempdir().unwrap();
    let pdf = fixture(dir.path());
    let (reader, mut v) = open(&pdf, cx);
    click(&mut v, "pdf-text-tool");
    let pos = v.debug_bounds("pdf-page-0").unwrap().origin + point(px(100.), px(130.));
    v.simulate_click(pos, Modifiers::default());
    v.update(|w, cx| w.draw(cx).clear(cx));
    reader
        .update(&mut v, |r, w, cx| {
            assert!(r.text_box.is_some());
            assert!(r.body.read(cx).focus_handle(cx).is_focused(w));
        })
        .unwrap();
    v.simulate_input("旁注");
    reader
        .update(&mut v, |r, _, cx| {
            assert_eq!(r.body.read(cx).value().as_ref(), "旁注")
        })
        .unwrap();
    click(&mut v, "save-page-text");
    let marks = Store::new(&pdf).read().unwrap();
    assert_eq!(marks[0].quote, "旁注");
    let Style::PageText(area) = marks[0].style else {
        panic!()
    };
    let (_, mut reopened) = open(&pdf, cx);
    let page = reopened.debug_bounds("pdf-page-0").unwrap();
    let delta = reopened.debug_bounds("page-text-0").unwrap().origin - area.bounds(page).origin;
    assert!(delta.x.abs() < px(1.) && delta.y.abs() < px(1.));
    click(&mut reopened, "page-text-0");
    reopened.update(|w, cx| w.draw(cx).clear(cx));
    reopened.simulate_keystrokes(&crate::shortcuts::command_default("ctrl-a"));
    reopened.simulate_input("更新文字");
    click(&mut reopened, "save-page-text");
    assert_eq!(Store::new(&pdf).read().unwrap()[0].quote, "更新文字");
    click(&mut reopened, "delete-mark");
    assert!(Store::new(&pdf).read().unwrap().is_empty());
}

#[gpui::test]
#[ignore = "requires packaged PDFium beside the test executable"]
fn pdf_ink_groups_strokes_and_reopens_at_a_different_zoom(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    cx.update(init);
    let dir = tempfile::tempdir().unwrap();
    let pdf = fixture(dir.path());
    let (_, mut view) = open(&pdf, cx);
    click(&mut view, "pdf-pen-tool");
    let bounds = view.debug_bounds("pdf-page-0").unwrap();
    let a = bounds.origin + point(px(70.), px(95.));
    let b = a + point(px(45.), px(50.));
    let c = b + point(px(45.), px(-30.));
    view.simulate_mouse_down(a, MouseButton::Left, Modifiers::default());
    view.simulate_mouse_move(b, Some(MouseButton::Left), Modifiers::default());
    view.simulate_mouse_up(c, MouseButton::Left, Modifiers::default());
    view.simulate_click(a + point(px(20.), px(10.)), Modifiers::default());
    let marks = Store::new(&pdf).read().unwrap();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].strokes[0].len(), 3);
    assert_eq!(marks[0].strokes[1].len(), 1);
    let Style::Ink(area) = marks[0].style else {
        panic!()
    };
    let (reader, mut reopened) = open(&pdf, cx);
    reader
        .update(&mut reopened, |r, _, cx| {
            r.zoom = 1.5;
            cx.notify();
        })
        .unwrap();
    for _ in 0..300 {
        reopened.executor().advance_clock(Duration::from_millis(15));
        reopened.run_until_parked();
        reopened.update(|w, cx| w.draw(cx).clear(cx));
        if reopened
            .debug_bounds("pdf-page-0")
            .is_some_and(|r| r.size.width > bounds.size.width)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let shown = reopened.debug_bounds("pdf-page-0").unwrap();
    assert!(shown.size.width > bounds.size.width);
    let hit = shown.origin + (b - bounds.origin) * (shown.size.width / bounds.size.width);
    reopened.simulate_click(hit, Modifiers::default());
    reader
        .update(&mut reopened, |r, _, _| {
            assert_eq!(r.editing, Some(0));
            assert_eq!(r.focused_area, Some(area));
        })
        .unwrap();
    click(&mut reopened, "delete-mark");
    assert!(Store::new(&pdf).read().unwrap().is_empty());
}
