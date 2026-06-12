# tool-helper

Small tool built in Golang to help with various terminal tools.

Provides a simple CLI with subcommands to help and complement other tools.

First subcommand should be for handling kitty sessions.

It should glob read '*.kitty-session' files in my home directory and use fzf to provide a selector UI to launch the sessions.

References:
<https://sw.kovidgoyal.net/kitty/sessions/>
<https://github.com/junegunn/fzf>
