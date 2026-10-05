//! Starts Wip's language server, `wip lsp`, for Zed.

use zed_extension_api as zed;

struct Wip;

impl zed::Extension for Wip {
    fn new() -> Self {
        Wip
    }

    fn language_server_command(
        &mut self,
        _server: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        let wip = worktree.which("wip").ok_or(
            "`wip` is not on your PATH: build the compiler with `cargo build --release` \
             and put its `target/release` on the PATH",
        )?;
        Ok(zed::Command {
            command: wip,
            args: vec!["lsp".to_string()],
            env: worktree.shell_env(),
        })
    }
}

zed::register_extension!(Wip);
