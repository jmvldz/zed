use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use smol::process::Command;

const PATH_PREFIX: &str = "__ZED_SWARM_PATH__";

fn parse_login_shell_path_output(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        line.trim()
            .strip_prefix(PATH_PREFIX)
            .map(|path| path.trim().to_string())
            .filter(|path| !path.is_empty())
    })
}

/// Import PATH from the user's login shell.
/// On macOS, GUI apps don't inherit the shell's PATH, so we need to
/// explicitly source it from the login shell.
#[cfg(target_os = "macos")]
pub async fn import_login_shell_path() -> Result<Option<String>> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let shell_name = Path::new(&shell)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");

    if shell_name != "zsh" && shell_name != "bash" {
        log::warn!(
            "Skipping login shell PATH import for unsupported shell: {}",
            shell_name
        );
        return Ok(None);
    }

    let mut command = Command::new(&shell);
    command
        .arg("-l")
        .arg("-c")
        .arg(format!("echo {PATH_PREFIX}$PATH"));

    let output = match smol::future::or(
        async {
            command.output().await
        },
        async {
            smol::Timer::after(Duration::from_secs(2)).await;
            Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout"))
        },
    ).await {
        Ok(output) => output,
        Err(e) => {
            log::warn!("Login shell PATH import failed: {}", e);
            return Ok(None);
        }
    };

    if !output.status.success() {
        log::warn!(
            "Login shell PATH import failed for shell {} with status {:?}",
            shell_name,
            output.status.code()
        );
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Some(path) = parse_login_shell_path_output(&stdout) {
        log::info!(
            "Imported login shell PATH from {} (len={})",
            shell_name,
            path.len()
        );
        return Ok(Some(path));
    }

    log::warn!(
        "Login shell PATH import returned unexpected output for shell {}",
        shell_name
    );
    Ok(None)
}

#[cfg(not(target_os = "macos"))]
pub async fn import_login_shell_path() -> Result<Option<String>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::parse_login_shell_path_output;

    #[test]
    fn test_parse_login_shell_path_output_extracts_path() {
        let output = "noise\n__ZED_SWARM_PATH__/opt/homebrew/bin:/usr/bin\n";
        let parsed = parse_login_shell_path_output(output);
        assert_eq!(parsed, Some("/opt/homebrew/bin:/usr/bin".to_string()));
    }

    #[test]
    fn test_parse_login_shell_path_output_ignores_empty() {
        let output = "__ZED_SWARM_PATH__\n";
        let parsed = parse_login_shell_path_output(output);
        assert_eq!(parsed, None);
    }
}
