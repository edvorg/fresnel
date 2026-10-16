//! PNG files of the terminals, for a look by a person: the styles, the colors, wide
//! characters and box characters of a shell, busybox `vi` and `top`, and the light theme
//! with a selection, a failed session and the question before a long paste.
//!
//! The pictures use the fonts of the system, as the app does: on macOS, Menlo Bold and a CJK
//! font. A system with none paints bold two times and a CJK character as a box.

use egui::Event;
use fresnel_core::terminal::{SelectKind, SessionState, Spot, TerminalView};
use fresnel_ui::{TerminalFonts, add_terminal_fonts};

use super::{SHELL, TOP, VI, focus, terminal, title, with_terminals};
use crate::data::{live, pods};
use crate::fake::{Window, grow_dock, in_one_frame, open_tab, view, window_with};
use crate::snapshots::{SCALE, save_clean};

/// The window at the scale of a Retina display, with the fonts of the system and a tall
/// dock with the tab of the first terminal.
fn window_of(terminals: &[TerminalView], theme: egui::Theme) -> Window {
    let view = with_terminals(view(vec![live("ams-prod", &pods())]), terminals);
    let mut window = window_with(view, theme, SCALE);
    add_terminal_fonts(&window.ctx, TerminalFonts::of_system().0);
    window.run();
    if let Some(first) = terminals.first() {
        open_tab(&mut window, &title(first));
    }
    grow_dock(&mut window, 300.0);
    window
}

/// A screen of each style, color and kind of character.
fn showcase() -> Vec<u8> {
    let mut screen = String::from("\x1b[1;32mroot@web-1\x1b[0m:\x1b[1;34m/srv/app\x1b[0m# printf …\r\n");
    screen.push_str(
        "\x1b[1mbold\x1b[0m \x1b[3mitalic\x1b[0m \x1b[4munderline\x1b[0m \x1b[9mstrike\x1b[0m \x1b[2mdim\x1b[0m \x1b[7minverse\x1b[0m \x1b[1;3;4mall three\x1b[0m\r\n",
    );
    for base in [30, 90] {
        for color in 0..8 {
            screen.push_str(&format!("\x1b[{}m color {} \x1b[0m", base + color, base + color));
        }
        for color in 0..8 {
            screen.push_str(&format!("\x1b[{}m  \x1b[0m", base + 10 + color));
        }
        screen.push_str("\r\n");
    }
    for index in 16..232 {
        screen.push_str(&format!("\x1b[48;5;{index}m "));
        if index % 72 == 15 {
            screen.push_str("\x1b[0m\r\n");
        }
    }
    screen.push_str("\x1b[0m\r\n");
    for step in 0..96 {
        let (red, blue) = (step * 255 / 95, 255 - step * 255 / 95);
        screen.push_str(&format!("\x1b[48;2;{red};80;{blue}m "));
    }
    screen.push_str("\x1b[0m\r\n");
    screen.push_str("wide: 中文字符 日本語のかな · emoji 😀🚀 · marks e\u{301} n\u{303} · Ж Ω ∑ → ✓\r\n");
    screen.push_str("┌──────────┬────────┐  ╔═════╗  ╭────╮  ┏━━━━━┓  ▁▂▃▄▅▆▇█  ░▒▓█\r\n");
    screen.push_str("│ cpu      │ \x1b[32m42 %\x1b[0m   │  ║ ok  ║  │ ⣿⣷ │  ┃ ┄┅┆ ┃  ▀▄▌▐▖▗▘▙▚▛\r\n");
    screen.push_str("├──────────┼────────┤  ╠═════╣  │ ⡇⢸ │  ┣━━━━━┫  ⣀⣤⣶⣿⣿⣶⣤⣀\r\n");
    screen.push_str("│ memory   │ \x1b[33m81 %\x1b[0m   │  ╚═════╝  ╰────╯  ┗━━━━━┛  ╱╲╳ ╌╎\r\n");
    screen.push_str("└──────────┴────────┘\r\n");
    screen.into_bytes()
}

#[test]
fn the_terminal_with_each_style_color_and_kind_of_character() {
    let web = terminal(1, "web-1");
    web.terminal.set_state(SessionState::Open);
    let mut window = window_of(std::slice::from_ref(&web), egui::Theme::Dark);
    web.terminal.feed(SHELL);
    web.terminal.feed(b"\x1b[H\x1b[2J");
    web.terminal.feed(&showcase());
    web.terminal.feed(b"\x1b]0;printf - web-1\x07\x1b[1;32mroot@web-1\x1b[0m# ");
    // No session takes the answers to the cursor requests of the recorded shell.
    web.terminal.drop_input();
    focus(&mut window);
    save_clean(&mut window, "terminal-shell");
}

#[test]
fn busybox_vi_and_top_in_the_terminal() {
    let (vi, top) = (terminal(1, "web-1"), terminal(2, "db-0"));
    for one in [&vi, &top] {
        one.terminal.set_state(SessionState::Open);
    }
    let mut window = window_of(&[vi.clone(), top.clone()], egui::Theme::Dark);
    // vi before the clear of the line of its state for `:q!`.
    let quit = VI.windows(3).position(|window| window == b":q!").expect("the quit of vi");
    let clear = VI.get(..quit).and_then(|before| before.windows(10).rposition(|window| window == b"\x1b[24;1H\x1b[K"));
    vi.terminal.feed(VI.get(..clear.expect("the clear of the line")).expect("vi"));
    focus(&mut window);
    save_clean(&mut window, "terminal-vi");
    open_tab(&mut window, &title(&top));
    top.terminal.feed(TOP);
    window.run();
    save_clean(&mut window, "terminal-top");
}

#[test]
fn the_light_theme_with_a_selection_a_failed_session_and_a_long_paste() {
    let web = terminal(1, "web-1");
    let mut window = window_of(std::slice::from_ref(&web), egui::Theme::Light);
    web.terminal.feed(SHELL);
    web.terminal.feed(&showcase());
    web.terminal.set_state(SessionState::Failed { reason: "no frame came for 15 s".to_owned() });
    // The click of the focus ends a selection, so the selection comes after it.
    focus(&mut window);
    web.terminal.select(SelectKind::Cells, Spot { row: 3, column: 10, right: false });
    web.terminal.select_to(Spot { row: 8, column: 30, right: true });
    in_one_frame(&mut window, [Event::Paste("y".repeat(200 * 1024))]);
    window.run();
    save_clean(&mut window, "terminal-light");
}
