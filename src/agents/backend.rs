use super::*;

/// Installs remote skill sources into the library.
pub(super) trait Backend {
    fn install(&self, directory: &Path, update: bool) -> Result<(), Error>;
}
pub(super) struct Rulesync {
    pub executable: String,
}
impl Default for Rulesync {
    fn default() -> Self {
        Self {
            executable: "rulesync".into(),
        }
    }
}
impl Backend for Rulesync {
    fn install(&self, directory: &Path, update: bool) -> Result<(), Error> {
        let mut args = vec!["install", "--config", "rulesync.jsonc"];
        if update {
            args.push("--update");
        }
        run_command(&self.executable, &args, directory)?;
        Ok(())
    }
}
