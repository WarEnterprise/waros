use core::sync::atomic::{AtomicBool, Ordering};

use x86_64::instructions::hlt;

use crate::auth::session;
use crate::display::console;
use crate::drivers::keyboard;
use crate::net;
use crate::task;

pub mod apps;
pub mod compositor;
pub mod cursor;
pub mod desktop;
pub mod font;
pub mod framebuffer;
pub mod mouse;
pub mod taskbar;
pub mod theme;
pub mod widgets;
pub mod window;

use self::apps::AppType;
use self::compositor::Compositor;

static GUI_ACTIVE: AtomicBool = AtomicBool::new(false);

#[must_use]
pub fn is_active() -> bool {
    GUI_ACTIVE.load(Ordering::Relaxed)
}

pub fn start_gui() {
    if GUI_ACTIVE.swap(true, Ordering::SeqCst) {
        crate::kprintln!("[WarOS] GUI is already active.");
        return;
    }

    let Some((width, height)) = console::with_console(|console| (console.width_pixels(), console.height_pixels())) else {
        GUI_ACTIVE.store(false, Ordering::Relaxed);
        crate::kprintln!("[WarOS] GUI unavailable: framebuffer console not initialized.");
        return;
    };

    mouse::init_mouse(width as i32, height as i32);
    console::set_rendering_enabled(false);

    let mut compositor = Compositor::new(width, height);
    compositor.open_app(AppType::Terminal);
    compositor.open_app(AppType::Quantum);
    compositor.open_app(AppType::SystemInfo);
    compositor.invalidate();

    let mut previous_left = false;

    loop {
        task::tick();
        let _ = net::poll();

        if !session::is_logged_in() {
            break;
        }

        let mouse_snapshot = mouse::take_snapshot();
        if mouse_snapshot.left && !previous_left {
            compositor.handle_mouse_down(mouse_snapshot.x, mouse_snapshot.y);
            compositor.invalidate();
        } else if mouse_snapshot.left && previous_left {
            compositor.handle_mouse_move(mouse_snapshot.x, mouse_snapshot.y);
            if mouse_snapshot.dirty {
                compositor.invalidate();
            }
        } else if !mouse_snapshot.left && previous_left {
            compositor.handle_mouse_up();
            compositor.invalidate();
        } else if mouse_snapshot.dirty {
            compositor.invalidate();
        }
        previous_left = mouse_snapshot.left;

        while let Some(key) = keyboard::read_char() {
            if key == 0x1B {
                console::set_rendering_enabled(true);
                console::clear_screen();
                GUI_ACTIVE.store(false, Ordering::SeqCst);
                return;
            }
            compositor.route_key_to_focused(key);
            compositor.invalidate();
        }

        compositor.render();

        if !compositor.needs_redraw() && !task::has_tasks() {
            hlt();
        }
    }

    console::set_rendering_enabled(true);
    console::clear_screen();
    GUI_ACTIVE.store(false, Ordering::SeqCst);
}
