use std::ffi::OsStr;

/// Windows process creation flag that prevents console applications from
/// allocating a visible console window when launched by the desktop GUI.
pub const WINDOWS_CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn std_background_command<S>(program: S) -> std::process::Command
where
    S: AsRef<OsStr>,
{
    let command = std::process::Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let mut command = command;
        command.creation_flags(WINDOWS_CREATE_NO_WINDOW);
        command
    }
    #[cfg(not(target_os = "windows"))]
    command
}

pub fn tokio_background_command<S>(program: S) -> tokio::process::Command
where
    S: AsRef<OsStr>,
{
    let command = tokio::process::Command::new(program);
    #[cfg(target_os = "windows")]
    {
        let mut command = command;
        command.creation_flags(WINDOWS_CREATE_NO_WINDOW);
        command
    }
    #[cfg(not(target_os = "windows"))]
    command
}

#[cfg(test)]
mod tests {
    use super::{std_background_command, tokio_background_command, WINDOWS_CREATE_NO_WINDOW};

    #[test]
    fn background_commands_preserve_program_and_arguments() {
        let mut std_command = std_background_command("background-helper");
        std_command.arg("--probe");
        assert_eq!(std_command.get_program(), "background-helper");
        assert_eq!(
            std_command
                .get_args()
                .map(|value| value.to_string_lossy().to_string())
                .collect::<Vec<_>>(),
            vec!["--probe"]
        );

        let mut tokio_command = tokio_background_command("background-helper");
        tokio_command.arg("--probe");
        assert_eq!(tokio_command.as_std().get_program(), "background-helper");
        assert_eq!(
            tokio_command
                .as_std()
                .get_args()
                .map(|value| value.to_string_lossy().to_string())
                .collect::<Vec<_>>(),
            vec!["--probe"]
        );
    }

    #[test]
    fn windows_no_window_flag_matches_win32_contract() {
        assert_eq!(WINDOWS_CREATE_NO_WINDOW, 0x0800_0000);
    }
}
