mod app;
mod app_actions;
mod backend;
mod detail_stack;
mod home;
mod logs;
mod metric_chart;
mod metric_details;
mod metrics;
mod model;
mod search;
mod shell;
mod theme;
mod traces;
mod widgets;

use gpui_kit::*;
actions!(native_app, [Quit]);
fn main() {
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus([Menu::new("OpenTelemetry Desktop Viewer")
            .items([MenuItem::action("Quit OpenTelemetry Desktop Viewer", Quit)])]);
        app::register_actions(cx);
        traces::register_actions(cx);
        cx.text_system()
            .add_fonts(vec![std::borrow::Cow::Borrowed(include_bytes!(
                "../assets/AtkinsonHyperlegibleNext.ttf"
            ))])
            .expect("Unable to load bundled Atkinson font");
        open_window(
            WindowOptions {
                window_min_size: Some(size(px(900.), px(600.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("OpenTelemetry Desktop Viewer".into()),
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1440.), px(900.)),
                    cx,
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| app::Viewer::new(window, cx)),
        )
        .expect("Unable to open OpenTelemetry Desktop Viewer");
        cx.activate(true);
    });
}
