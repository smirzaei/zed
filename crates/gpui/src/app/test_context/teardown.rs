use crate::{Empty, TestAppContext};

#[gpui::test]
fn test_window_test_without_leaks_passes(cx: &mut TestAppContext) {
    let (view, _) = cx.add_window_view(|_, _| Empty);
    drop(view);
}

#[gpui::test]
#[should_panic(expected = "Exited with leaked handles")]
fn test_window_test_leaking_view_fails(cx: &mut TestAppContext) {
    let (view, _) = cx.add_window_view(|_, _| Empty);
    std::mem::forget(view);
}

#[gpui::test]
#[should_panic(expected = "Exited with leaked handles")]
async fn test_async_window_test_leaking_view_fails(cx: &mut TestAppContext) {
    let (view, _) = cx.add_window_view(|_, _| Empty);
    std::mem::forget(view);
}
