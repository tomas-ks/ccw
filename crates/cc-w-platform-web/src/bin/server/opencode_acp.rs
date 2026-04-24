use std::{
    env,
    path::PathBuf,
    process::{Child, Command, Stdio},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpencodeAcpConfig {
    pub executable: PathBuf,
    pub working_directory: Option<PathBuf>,
    pub config_path: Option<PathBuf>,
    pub hostname: String,
    pub port: u16,
    pub pure: bool,
}

impl Default for OpencodeAcpConfig {
    fn default() -> Self {
        Self {
            executable: PathBuf::from("opencode"),
            working_directory: None,
            config_path: None,
            hostname: "127.0.0.1".to_owned(),
            port: 0,
            pure: true,
        }
    }
}

impl OpencodeAcpConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Some(value) = env::var_os("CC_W_OPENCODE_EXECUTABLE") {
            config.executable = PathBuf::from(value);
        }
        if let Some(value) = env::var_os("CC_W_OPENCODE_WORKDIR") {
            config.working_directory = Some(PathBuf::from(value));
        }
        if let Some(value) = env::var_os("CC_W_OPENCODE_CONFIG") {
            config.config_path = Some(PathBuf::from(value));
        }
        if let Ok(value) = env::var("CC_W_OPENCODE_ACP_HOSTNAME") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                config.hostname = trimmed.to_owned();
            }
        }
        if let Ok(value) = env::var("CC_W_OPENCODE_ACP_PORT") {
            if let Ok(port) = value.trim().parse::<u16>() {
                config.port = port;
            }
        }
        if let Ok(value) = env::var("CC_W_OPENCODE_ACP_PURE") {
            let trimmed = value.trim();
            config.pure = !matches!(
                trimmed.to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            );
        }

        config
    }

    pub fn build_command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command.arg("acp");
        if self.pure {
            command.arg("--pure");
        }
        command.arg("--hostname");
        command.arg(&self.hostname);
        command.arg("--port");
        command.arg(self.port.to_string());
        if let Some(working_directory) = &self.working_directory {
            command.current_dir(working_directory);
        }
        if let Some(config_path) = &self.config_path {
            command.env("OPENCODE_CONFIG", config_path);
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    pub fn spawn(&self) -> Result<Child, std::io::Error> {
        self.build_command().spawn()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acp_command_includes_hostname_port_and_pure_by_default() {
        let config = OpencodeAcpConfig::default();
        let command = config.build_command();
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            args,
            vec![
                "acp".to_owned(),
                "--pure".to_owned(),
                "--hostname".to_owned(),
                "127.0.0.1".to_owned(),
                "--port".to_owned(),
                "0".to_owned(),
            ]
        );
    }

    #[test]
    fn acp_config_reads_env_overrides() {
        let config = OpencodeAcpConfig {
            executable: PathBuf::from("opencode"),
            working_directory: Some(PathBuf::from("/tmp")),
            config_path: Some(PathBuf::from("/tmp/opencode.json")),
            hostname: "localhost".to_owned(),
            port: 1234,
            pure: false,
        };

        let command = config.build_command();
        assert_eq!(
            command.get_current_dir().map(PathBuf::from),
            Some(PathBuf::from("/tmp"))
        );
        assert_eq!(
            command.get_envs().find_map(|(key, value)| {
                (key == "OPENCODE_CONFIG")
                    .then(|| value.map(|value| value.to_string_lossy().into_owned()))
                    .flatten()
            }),
            Some("/tmp/opencode.json".to_owned())
        );
    }
}
