pub mod files;
pub mod quantum;
pub mod sysinfo;
pub mod terminal;

use self::files::FileBrowserState;
use self::quantum::QuantumMonitorState;
use self::sysinfo::SystemInfoState;
use self::terminal::TerminalState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppType {
    Terminal,
    Quantum,
    SystemInfo,
    FileBrowser,
}

impl AppType {
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Terminal => "Terminal",
            Self::Quantum => "Quantum Monitor",
            Self::SystemInfo => "System Info",
            Self::FileBrowser => "Files",
        }
    }

    #[must_use]
    pub fn launcher_label(self) -> &'static str {
        match self {
            Self::Terminal => "Terminal",
            Self::Quantum => "Quantum",
            Self::SystemInfo => "Info",
            Self::FileBrowser => "Files",
        }
    }

    #[must_use]
    pub fn default_geometry(self) -> (i32, i32, usize, usize) {
        match self {
            Self::Terminal => (72, 64, 720, 440),
            Self::Quantum => (820, 72, 380, 250),
            Self::SystemInfo => (840, 344, 360, 220),
            Self::FileBrowser => (120, 120, 420, 260),
        }
    }
}

pub enum AppKind {
    Terminal(TerminalState),
    Quantum(QuantumMonitorState),
    SystemInfo(SystemInfoState),
    FileBrowser(FileBrowserState),
}

impl AppKind {
    #[must_use]
    pub fn new(app_type: AppType, width: usize, height: usize) -> Self {
        match app_type {
            AppType::Terminal => Self::Terminal(TerminalState::new(width, height)),
            AppType::Quantum => Self::Quantum(QuantumMonitorState::new()),
            AppType::SystemInfo => Self::SystemInfo(SystemInfoState::new()),
            AppType::FileBrowser => Self::FileBrowser(FileBrowserState::new()),
        }
    }

    #[must_use]
    pub fn app_type(&self) -> AppType {
        match self {
            Self::Terminal(_) => AppType::Terminal,
            Self::Quantum(_) => AppType::Quantum,
            Self::SystemInfo(_) => AppType::SystemInfo,
            Self::FileBrowser(_) => AppType::FileBrowser,
        }
    }

    pub fn render(&mut self, buffer: &mut [u32], width: usize, height: usize) {
        match self {
            Self::Terminal(state) => state.render(buffer, width, height),
            Self::Quantum(state) => state.render(buffer, width, height),
            Self::SystemInfo(state) => state.render(buffer, width, height),
            Self::FileBrowser(state) => state.render(buffer, width, height),
        }
    }

    #[must_use]
    pub fn handle_key(&mut self, key: u8) -> bool {
        match self {
            Self::Terminal(state) => state.handle_key(key),
            Self::Quantum(_) | Self::SystemInfo(_) | Self::FileBrowser(_) => false,
        }
    }
}
