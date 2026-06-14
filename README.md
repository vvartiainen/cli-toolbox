# cli-toolbox

Small Go CLI for a few daily workflow helpers.

Command parsing and help output are handled with [Kong](https://github.com/alecthomas/kong).

## Requirements

- Go
- `fzf`
- `kitty`
- `aws`
- `just` (for development)

## Commands

### `aws`

AWS helpers.

#### `aws profile`

Pick an AWS profile with `fzf`, log in to AWS SSO if needed, and print the `AWS_PROFILE` export for your shell.

Use it via `eval` so the exported variable is applied in your current shell:

```sh
eval "$(cli-toolbox aws profile)"
```

### `kitty`

Kitty helpers.

#### `kitty select-session`

Choose a `*.kitty-session` file from your home directory and switch Kitty to it.

```sh
cli-toolbox kitty select-session
```

### `ssh`

SSH helpers.

#### `ssh connect`

Choose an SSH host from your config and connect to it with Kitty's SSH kitten.

```sh
cli-toolbox ssh connect
```

## Common Development Commands

```sh
just
just build
just test
just fmt
```

## Build

```sh
just build
```

## Run

Invoke the built binary directly:

```sh
./bin/cli-toolbox <arguments>
```
