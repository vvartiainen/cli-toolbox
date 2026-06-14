# cli-toolbox

Small Go CLI for a few daily workflow helpers:

- pick an AWS profile and print an `AWS_PROFILE` export
- pick a kitty session file and open it
- pick an SSH host from `~/.ssh/config` and connect with kitty's SSH kitten

## Commands

```text
cli-toolbox aws profile
cli-toolbox kitty-session
cli-toolbox ssh
```

Command parsing and help output are handled with [Kong](https://github.com/alecthomas/kong).

### `aws profile`

Selects an AWS profile with `fzf`. If the profile uses AWS SSO, it runs `aws sso login` when needed, then prints:

```sh
export AWS_PROFILE='profile-name'
```

Use it like this:

```sh
eval "$(cli-toolbox aws profile)"
```

### `kitty-session`

Finds `*.kitty-session` files in your home directory, lets you choose one with `fzf`, and opens it through kitty remote control.

### `ssh`

Reads hosts from `~/.ssh/config`, lets you choose one with `fzf`, and connects with `kitten ssh`.

## Common Development Commands

If you have [`just`](https://github.com/casey/just) installed, the repository now includes a `justfile` for the common workflows:

```sh
just
just build
just test
just fmt
```

## Build

With `just`:

```sh
just build
```

Without `just`:

```sh
go build -o cli-toolbox ./cmd/cli-toolbox
```

## Run

Invoke the built binary directly:

```sh
./bin/cli-toolbox aws profile
./bin/cli-toolbox kitty-session
./bin/cli-toolbox ssh
```

## Requirements

- Go
- `fzf`
- `kitty` or `kitten`
- AWS CLI for `aws profile`
