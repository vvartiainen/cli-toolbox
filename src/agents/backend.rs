use super::*;

pub(super) trait Backend {
    fn generate(&self, target: &str, source: &Path, output: &Path) -> Result<(), Error>;
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
    fn generate(&self, target: &str, source: &Path, output: &Path) -> Result<(), Error> {
        // macOS temporary directories can be reached through /var or /private/var.
        // Rulesync compares resolved paths with its CWD when checking boundaries.
        let source = source.canonicalize()?;
        let output = output.canonicalize()?;
        let parent = source
            .parent()
            .ok_or_else(|| Error::msg("staging input has no parent"))?;
        let config = parent.join("rulesync.jsonc");
        write_json(
            &config,
            &json!({"targets":[target],"features":["mcp","skills"],"inputRoots":[source],"outputRoots":[output],"delete":false,"global":false,"sources":[]}),
        )?;
        run_command(
            &self.executable,
            &[
                "generate",
                "--config",
                "rulesync.jsonc",
                "--targets",
                target,
                "--features",
                "mcp,skills",
                "--input-roots",
                utf8(&source)?,
            ],
            parent,
        )?;
        Ok(())
    }
    fn install(&self, directory: &Path, update: bool) -> Result<(), Error> {
        let mut args = vec!["install", "--config", "rulesync.jsonc"];
        if update {
            args.push("--update");
        }
        run_command(&self.executable, &args, directory)?;
        Ok(())
    }
}
fn utf8(path: &Path) -> Result<&str, Error> {
    path.to_str()
        .ok_or_else(|| Error::msg("Rulesync requires UTF-8 paths"))
}
