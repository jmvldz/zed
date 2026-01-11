mod assets;
mod env;
mod swarm_window;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use gpui::{
    actions, px, size, App, AppContext, Bounds, KeyBinding, WindowBounds, WindowOptions,
    colors::{Colors, GlobalColors},
};
use log::LevelFilter;
use reqwest_client::ReqwestClient;
use settings::Settings;
use simplelog::SimpleLogger;
use swarm_chat::message_input::{SendMessage, OpenFilePicker};
use theme::ThemeSettings;

use crate::assets::{Assets, load_embedded_fonts};
use crate::swarm_window::SwarmWindow;

actions!(zed_swarm, [Quit, NewConversation, OpenCommitPicker]);

#[derive(Parser)]
#[command(name = "zed-swarm")]
#[command(author, version, about = "Zed Swarm - AI Chat Interface", long_about = None)]
struct Args {
    /// Path to the repository to open
    #[arg(long)]
    repo: Option<PathBuf>,

    /// Resume a previous session by ID
    #[arg(long)]
    session: Option<String>,

    /// Theme name to use
    #[arg(long, default_value = "One Dark")]
    theme: String,
}

fn main() {
    SimpleLogger::init(LevelFilter::Info, Default::default())
        .expect("could not initialize logger");

    // Import PATH from login shell before starting the app.
    // On macOS, GUI apps don't inherit the shell's PATH, which is needed
    // for finding the `codex` binary.
    smol::block_on(async {
        match env::import_login_shell_path().await {
            Ok(Some(path)) => {
                // SAFETY: We're setting PATH before any threads are spawned,
                // and we're the only code modifying environment at this point.
                unsafe { std::env::set_var("PATH", &path) };
                log::info!("Updated PATH from login shell (len={})", path.len());
            }
            Ok(None) => {
                log::debug!("No PATH import needed or available");
            }
            Err(e) => {
                log::warn!("Failed to import login shell PATH: {}", e);
            }
        }
    });

    menu::init();
    let args = Args::parse();

    gpui::Application::new().with_assets(Assets).run(move |cx| {
        if let Err(error) = init_app(cx, args) {
            log::error!("Failed to initialize Zed Swarm: {}", error);
            cx.quit();
        }
    });
}

fn init_app(cx: &mut App, args: Args) -> Result<()> {
    load_embedded_fonts(cx)?;

    cx.set_global(GlobalColors(Arc::new(Colors::default())));

    let http_client = ReqwestClient::user_agent("zed-swarm")?;
    cx.set_http_client(Arc::new(http_client));

    settings::init(cx);
    theme::init(theme::LoadThemes::All(Box::new(Assets)), cx);

    let mut theme_settings = ThemeSettings::get_global(cx).clone();
    theme_settings.theme =
        theme::ThemeSelection::Static(settings::ThemeName(args.theme.clone().into()));
    ThemeSettings::override_global(theme_settings, cx);

    editor::init(cx);
    swarm_chat::init(cx);
    swarm_file_picker::init(cx);
    swarm_store::init(cx);

    init_actions(cx);

    let repo_path = args.repo.clone();
    let session_id = args.session.clone();

    let window_size = size(px(900.), px(700.));
    let bounds = Bounds::centered(None, window_size, cx);

    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Zed Swarm".into()),
                appears_transparent: true,
                traffic_light_position: Some(gpui::point(px(9.0), px(9.0))),
            }),
            ..Default::default()
        },
        move |window, cx| {
            theme::setup_ui_font(window, cx);
            cx.new(|cx| SwarmWindow::new(repo_path, session_id, window, cx))
        },
    )?;

    cx.activate(true);
    Ok(())
}

fn init_actions(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());

    // Bind essential keys explicitly (we don't load Zed's full keymap since
    // it contains actions like debugger::* that aren't available in zed_swarm)
    cx.bind_keys([
        // Editor basics
        KeyBinding::new("backspace", editor::actions::Backspace, Some("Editor")),
        KeyBinding::new("shift-backspace", editor::actions::Backspace, Some("Editor")),
        KeyBinding::new("delete", editor::actions::Delete, Some("Editor")),
        KeyBinding::new("left", editor::actions::MoveLeft, Some("Editor")),
        KeyBinding::new("right", editor::actions::MoveRight, Some("Editor")),
        KeyBinding::new("up", editor::actions::MoveUp, Some("Editor")),
        KeyBinding::new("down", editor::actions::MoveDown, Some("Editor")),
        KeyBinding::new("enter", editor::actions::Newline, Some("Editor")),
        KeyBinding::new("home", editor::actions::MoveToBeginning, Some("Editor")),
        KeyBinding::new("end", editor::actions::MoveToEnd, Some("Editor")),
        KeyBinding::new("cmd-a", editor::actions::SelectAll, Some("Editor")),
        KeyBinding::new("cmd-c", editor::actions::Copy, Some("Editor")),
        KeyBinding::new("cmd-v", editor::actions::Paste, Some("Editor")),
        KeyBinding::new("cmd-x", editor::actions::Cut, Some("Editor")),
        KeyBinding::new("cmd-z", editor::actions::Undo, Some("Editor")),
        KeyBinding::new("cmd-shift-z", editor::actions::Redo, Some("Editor")),
        // App actions
        KeyBinding::new("cmd-q", Quit, None),
        // MessageInput: Enter sends, Shift+Enter for newline
        KeyBinding::new("enter", SendMessage, Some("MessageInput")),
        KeyBinding::new("cmd-enter", SendMessage, Some("MessageInput")),
        KeyBinding::new("shift-enter", editor::actions::Newline, Some("MessageInput")),
        KeyBinding::new("cmd-p", OpenFilePicker, Some("MessageInput")),
    ]);
}
