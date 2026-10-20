# Run from the repository root (or an agent worktree) so /workspace/app is
# that checkout:
#
#   nix run ./sandbox
#   nix run ./sandbox#ssh
#
# Or: just sandbox / just sandbox-ssh. One VM per agent worktree, in the
# background with herdr: cli-toolbox agent start / sync / done.
#
# Exit the vfkit console with Ctrl-a x. Guest home persists in:
#
#   .agent-sandbox/home.img

{
  description = "cli-toolbox agent-sandbox workspace";

  inputs.agent-sandbox.url = "git+ssh://git@github.com/vvartiainen/agent-sandbox-setup";

  outputs =
    { agent-sandbox, ... }:
    agent-sandbox.lib.mkFlake {
      name = "cli-toolbox";
      modules = [
        # Rust environment for this crate.
        (
          { pkgs, ... }:
          {
            environment.systemPackages = with pkgs; [
              cargo
              clippy
              just
              rust-analyzer
              rustc
              rustfmt
            ];
            environment.variables.CARGO_HOME = "/home/agent/.cargo";

            # Keep Linux build output out of the host's target/.
            agentSandbox.guestPaths = [ "app/target" ];

            agentSandbox.init.steps.cargo-fetch = {
              runtimeInputs = [ pkgs.cargo ];
              inputs = [
                "Cargo.toml"
                "Cargo.lock"
              ];
              script = "cargo fetch";
            };
          }
        )
        {
          agentSandbox = {
            vcpu = 4;
            memoryMB = 4096;
            # This checkout, at /workspace/app in the guest.
            workspaces.app = ".";

            # nix run ./sandbox#ssh
            ssh = {
              enable = true;
              hostPort = 2222;
              authorizedKeys = [
                ""
              ];
            };

            # cli-toolbox agent start adds the VM as a herdr machine.
            herdr.enable = true;

            ai = {
              codex.enable = true;
              copilot.enable = true;
            };

            secrets = {
              install = [ "GITHUB_PACKAGES_TOKEN" ];
              agent = [
                "AGENT_GITHUB_TOKEN"
                "COPILOT_GITHUB_TOKEN"
              ];
            };
          };
        }
      ];
    };
}
