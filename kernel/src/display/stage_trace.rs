use core::sync::atomic::{AtomicU8, Ordering};

use crate::display::console::{self, FramebufferConsole};
use crate::display::font;
pub const DEBUG_STAGE_STOP: Option<u8> = None;

static LAST_STAGE_ID: AtomicU8 = AtomicU8::new(0xFF);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum VisualStage {
    BuildTag = 0,
    AfterAuthReady = 1,
    AfterBootHeapMark = 2,
    BeforeAuthSelect = 3,
    AfterAuthSelect = 4,
    BeforePrefCall = 5,
    PrefFunctionEntry = 6,
    FirstRenderBegin = 7,
    FirstRenderDone = 8,
    WaitingInput = 9,
    BeforePrefReturnTest = 10,
}

#[derive(Clone, Copy)]
struct StageSpec {
    id: u8,
    stage_line: &'static str,
    label_line: &'static str,
    background: u32,
    foreground: u32,
    accent: u32,
}

pub fn show_build_tag() {
    draw_stage(VisualStage::BuildTag, true);
}

pub fn mark(stage: VisualStage) {
    draw_stage(stage, false);
}

fn draw_stage(stage: VisualStage, force: bool) {
    if !console::screen_owner_is(console::ScreenOwner::Boot) {
        crate::serial_println!(
            "[SCREEN] stage-trace ignored stage={:?} owner={:?}",
            stage,
            console::current_screen_owner()
        );
        return;
    }
    let spec = stage_spec(stage);
    let previous = LAST_STAGE_ID.swap(spec.id, Ordering::Relaxed);
    if !force && previous == spec.id {
        return;
    }

    let _ = console::with_raw_console(|console| render_stage(console, spec));

    if DEBUG_STAGE_STOP == Some(spec.id) {
        crate::arch::x86_64::hlt_loop();
    }
}

fn render_stage(console: &mut FramebufferConsole, spec: StageSpec) {
    if spec.id == VisualStage::BuildTag as u8 {
        render_build_identity(console, spec);
        return;
    }

    let width = console.width_pixels();
    let height = console.height_pixels();
    let banner_height = 28usize;
    let footer_height = 22usize;
    let stage_height = font::FONT_HEIGHT_PIXELS.saturating_mul(4);
    let stage_y = height
        .saturating_sub(stage_height.saturating_add(font::FONT_HEIGHT_PIXELS.saturating_mul(5)))
        / 2;
    let label_y = stage_y + font::FONT_HEIGHT_PIXELS * 5;

    console.raw_begin_batch();
    console.raw_clear_screen_with_color(spec.background);
    fill_rect(console, 0, 0, width, banner_height, spec.accent);
    fill_rect(
        console,
        0,
        height.saturating_sub(footer_height),
        width,
        footer_height,
        spec.accent,
    );
    draw_centered_text(console, 6, crate::BUILD_TAG, 1, spec.background);
    draw_centered_text(console, stage_y, spec.stage_line, 4, spec.foreground);
    draw_centered_text(console, label_y, spec.label_line, 2, spec.foreground);
    console.raw_end_batch();
}

fn render_build_identity(console: &mut FramebufferConsole, spec: StageSpec) {
    let width = console.width_pixels();
    let height = console.height_pixels();
    let title_y = height / 5;
    let subtitle_y = title_y + font::FONT_HEIGHT_PIXELS * 6;
    let detail_y = subtitle_y + font::FONT_HEIGHT_PIXELS * 3;
    let footer_y = height.saturating_sub(font::FONT_HEIGHT_PIXELS * 3);

    console.raw_begin_batch();
    console.raw_clear_screen_with_color(0x000C111B);
    fill_rect(console, 0, 0, width, 22, spec.accent);
    fill_rect(console, 0, height.saturating_sub(10), width, 10, 0x00161D29);
    fill_rect(console, width.saturating_sub(18), 0, 18, height, 0x00161D29);

    draw_text(console, 18, 4, "WAR ENTERPRISE", 1, 0x000C111B);
    draw_centered_text(console, title_y, spec.stage_line, 5, 0x00E6EDF3);
    draw_centered_text(
        console,
        subtitle_y,
        spec.label_line,
        1,
        0x0056D4DD,
    );
    draw_centered_text(console, detail_y, crate::BUILD_TAG, 1, 0x008B949E);
    draw_centered_text(console, footer_y, "BOOT DIAGNOSTICS FOLLOW", 1, 0x008B949E);
    console.raw_end_batch();
}

fn fill_rect(
    console: &mut FramebufferConsole,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    color: u32,
) {
    let max_x = x.saturating_add(width).min(console.width_pixels());
    let max_y = y.saturating_add(height).min(console.height_pixels());
    for py in y..max_y {
        for px in x..max_x {
            console.write_pixel(px, py, color);
        }
    }
}

fn draw_centered_text(
    console: &mut FramebufferConsole,
    y: usize,
    text: &'static str,
    scale: usize,
    color: u32,
) {
    if scale == 0 {
        return;
    }

    let text_width = text
        .as_bytes()
        .len()
        .saturating_mul(font::FONT_WIDTH)
        .saturating_mul(scale);
    let x = console.width_pixels().saturating_sub(text_width) / 2;
    draw_text(console, x, y, text, scale, color);
}

fn draw_text(
    console: &mut FramebufferConsole,
    mut x: usize,
    y: usize,
    text: &'static str,
    scale: usize,
    color: u32,
) {
    for byte in text.bytes() {
        draw_char(console, x, y, byte, scale, color);
        x = x.saturating_add(font::FONT_WIDTH.saturating_mul(scale));
    }
}

fn draw_char(
    console: &mut FramebufferConsole,
    x: usize,
    y: usize,
    byte: u8,
    scale: usize,
    color: u32,
) {
    let glyph = font::glyph(char::from(byte));
    for (row_index, row) in glyph.raster().iter().enumerate() {
        for (column_index, intensity) in row.iter().copied().enumerate() {
            if intensity == 0 {
                continue;
            }
            let pixel_x = x.saturating_add(column_index.saturating_mul(scale));
            let pixel_y = y.saturating_add(row_index.saturating_mul(scale));
            fill_rect(console, pixel_x, pixel_y, scale, scale, color);
        }
    }
}

const fn stage_spec(stage: VisualStage) -> StageSpec {
    match stage {
        VisualStage::BuildTag => StageSpec {
            id: 0,
            stage_line: "WAROS",
            label_line: "HYBRID QUANTUM-CLASSICAL OPERATING SYSTEM",
            background: 0x001B2A,
            foreground: 0xFFFFFF,
            accent: 0x00BF63,
        },
        VisualStage::AfterAuthReady => StageSpec {
            id: 1,
            stage_line: "STAGE 1",
            label_line: "AFTER_AUTH_READY",
            background: 0x2D0B0B,
            foreground: 0xFFF3F3,
            accent: 0xFF5A5A,
        },
        VisualStage::AfterBootHeapMark => StageSpec {
            id: 2,
            stage_line: "STAGE 2",
            label_line: "AFTER_BOOT_HEAP_MARK",
            background: 0x2B1A00,
            foreground: 0xFFF4D6,
            accent: 0xF3A712,
        },
        VisualStage::BeforeAuthSelect => StageSpec {
            id: 3,
            stage_line: "STAGE 3",
            label_line: "BEFORE_AUTH_SELECT",
            background: 0x231F00,
            foreground: 0xFFFBD1,
            accent: 0xE3D26F,
        },
        VisualStage::AfterAuthSelect => StageSpec {
            id: 4,
            stage_line: "STAGE 4",
            label_line: "AFTER_AUTH_SELECT",
            background: 0x0A2333,
            foreground: 0xE8F7FF,
            accent: 0x55D6FF,
        },
        VisualStage::BeforePrefCall => StageSpec {
            id: 5,
            stage_line: "STAGE 5",
            label_line: "BEFORE_PREF_CALL",
            background: 0x12213A,
            foreground: 0xF0F4FF,
            accent: 0x6EA8FE,
        },
        VisualStage::PrefFunctionEntry => StageSpec {
            id: 6,
            stage_line: "STAGE 6",
            label_line: "PREF_FUNCTION_ENTRY",
            background: 0x102A1B,
            foreground: 0xF1FFF7,
            accent: 0x42D392,
        },
        VisualStage::FirstRenderBegin => StageSpec {
            id: 7,
            stage_line: "STAGE 7",
            label_line: "FIRST_RENDER_BEGIN",
            background: 0x1B1230,
            foreground: 0xF7F1FF,
            accent: 0xB388FF,
        },
        VisualStage::FirstRenderDone => StageSpec {
            id: 8,
            stage_line: "STAGE 8",
            label_line: "FIRST_RENDER_DONE",
            background: 0x2A1022,
            foreground: 0xFFF2FA,
            accent: 0xFF7AC6,
        },
        VisualStage::WaitingInput => StageSpec {
            id: 9,
            stage_line: "STAGE 9",
            label_line: "WAITING_INPUT",
            background: 0x101010,
            foreground: 0xF8F8F8,
            accent: 0xFFFFFF,
        },
        VisualStage::BeforePrefReturnTest => StageSpec {
            id: 10,
            stage_line: "STAGE 5A",
            label_line: "BEFORE_RETURN_TEST",
            background: 0x240018,
            foreground: 0xFFF0FB,
            accent: 0xFF4DB8,
        },
    }
}
