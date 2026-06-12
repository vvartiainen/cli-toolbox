# NIX-INSTRUCTIONS

## Consuming `cli-toolbox` from a `nix-darwin` repo

This repository exposes a flake package at:

- `packages.${system}.default`

You can consume it from another flake, such as a `nix-darwin` + Home Manager config.

### 1. Add it as a flake input

```nix
inputs.cli-toolbox.url = "github:YOUR_GITHUB_USER/cli-toolbox";
```

### 2. Add the package to Home Manager

In a Home Manager module:

```nix
{ pkgs, inputs, ... }:

{
  home.packages = [
    inputs.cli-toolbox.packages.${pkgs.system}.default
  ];
}
```

This makes the `cli-toolbox` binary available in your user profile.

### 3. Rebuild your system

```sh
darwin-rebuild switch --flake .#YOUR_HOSTNAME
```

### 4. About `vendorHash`

The flake sets:

```nix
vendorHash = null;
```

That is intentional for this project. `cli-toolbox` currently has no external Go module dependencies to vendor, so there is no concrete vendor hash to fill in.

### Runtime dependencies

This package only installs the `cli-toolbox` binary itself. Runtime tools used by some commands are expected to be installed separately in your environment, for example:

- `fzf`
- `kitty`
- `aws`

That matches setups where those tools are already provided via Homebrew, Home Manager, or some other package source.

### Optional local usage

You can also run the package directly from this repo:

```sh
nix run
```
