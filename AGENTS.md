# cli-toolbox

This is a small CLI toolbox / helper written with modern Golang.

The purpose is to extend and help with the functionality of some CLIs I use, for example:

- Read .kitty-session files in home directory and select one of those to change to
- Read SSH host configurations and select one to connect to
- Read AWS profiles and login to one if needed and take it into use

## Generic guidelines

1. Use modern Golang features  (>=1.26.0)
2. Prefer stdlib features to adding new dependencies
3. Well established dependencies can be used, like Kong for CLI command and flag parsing

## Development workflow

1. Start by writing simple happy path tests for the feature
2. Write the implementation
3. Keep iterating the implementation until the tests pass
4. Run go formatting and linting commands and fix the issues
