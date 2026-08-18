use komorebi::ui::App;

fn main() {
    env_logger::init();
    let app = gpui::Application::new();
    app.run(|cx| {
        use gpui::AppContext;
        gpui_component::init(cx);
        let bounds =
            gpui::Bounds::centered(None, gpui::size(gpui::px(1280.0), gpui::px(820.0)), cx);
        cx.spawn(async move |cx| {
            cx.open_window(
                gpui::WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                    // Custom title bar rendered inside App; this gives GPUI a
                    // transparent title bar with the platform window controls.
                    titlebar: Some(gpui_component::TitleBar::title_bar_options()),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| App::new(window, cx));
                    cx.new(|cx| gpui_component::Root::new(view, window, cx))
                },
            )?;
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}
