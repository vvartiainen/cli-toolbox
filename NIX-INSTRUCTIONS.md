# NIX-INSTRUCTIONS

## Consuming `cli-toolbox` from a `nix-darwin` repo

This repository exposes a flake package at:

- `packages.${system}.default`

You can consume it from another flake, such as a `nix-darwin` + Home Manager config.

### 1. Add it as a flake input

```nix
inputs.cli-toolbox.url = "github:vvartiainen/cli-toolbox";
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

This makes the `cli-toolbox` binary available in your user profile. The package also installs bash, zsh, and fish completions.

### 3. Rebuild your system

```sh
darwin-rebuild switch --flake .#YOUR_HOSTNAME
```

### 4. About `Cargo.lock`

The flake builds with `rustPlatform.buildRustPackage` and pins crate versions from `Cargo.lock`. Commit lockfile changes when you add, remove, or update Rust dependencies.

### Runtime dependencies

This package only installs the `cli-toolbox` binary itself. Runtime tools used by some commands are expected to be installed separately in your environment, for example:

- `fzf` (optional; a built-in picker is used if it is missing)
- `kitty`
- `aws`
- `git`

That matches setups where those tools are already provided via Homebrew, Home Manager, or some other package source.

### Optional local usage

You can also run the package directly from this repo:

```sh
nix run
```
