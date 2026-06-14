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

## Build

```sh
go build -o cli-toolbox ./cmd/cli-toolbox
```

## Run

Run without installing:

```sh
go run ./cmd/cli-toolbox aws profile
go run ./cmd/cli-toolbox kitty-session
go run ./cmd/cli-toolbox ssh
```

Run the built binary:

```sh
./cli-toolbox aws profile
./cli-toolbox kitty-session
./cli-toolbox ssh
```

## Requirements

- Go
- `fzf`
- `kitty` or `kitten`
- AWS CLI for `aws profile`
