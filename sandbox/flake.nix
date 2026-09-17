# Run from the repository root so /workspace is this project:
#
#   nix run ./sandbox
#   nix run ./sandbox#ssh
#
# Or: just sandbox / just sandbox-ssh
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
        {
          agentSandbox = {
            vcpu = 4;
            memoryMB = 4096;
            workspace = "."; # Shared with the guest at /workspace.
            rust.enable = true;

            # nix run ./sandbox#ssh
            ssh = {
              enable = true;
              hostPort = 2222;
              authorizedKeys = [
                ""
              ];
            };

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
        (
          { pkgs, ... }:
          {
            environment.systemPackages = [ pkgs.just ];
          }
        )
      ];
    };
}
