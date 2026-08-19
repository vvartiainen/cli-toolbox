package cli

import (
	"errors"
	"fmt"
	"io"
	"os"

	"cli-toolbox/internal/cli/awscmd"
	"cli-toolbox/internal/cli/gitcmd"
	"cli-toolbox/internal/cli/kittycmd"
	"cli-toolbox/internal/cli/sshcmd"

	"github.com/alecthomas/kong"
	kongcompletion "github.com/jotaen/kong-completion"
)

type rootCommand struct {
	AWS        awscmd.Command            `cmd:"" name:"aws" help:"AWS helpers."`
	Git        gitcmd.Command            `cmd:"" name:"git" help:"Git helpers."`
	Kitty      kittycmd.Command          `cmd:"" name:"kitty" help:"Kitty helpers."`
	SSH        sshcmd.Command            `cmd:"" name:"ssh" help:"SSH helpers."`
	Completion kongcompletion.Completion `cmd:"" help:"Generate shell completion scripts."`
}

func newRootCommand(stdout, stderr io.Writer) *rootCommand {
	return &rootCommand{
		AWS:   awscmd.New(stdout, stderr),
		Git:   gitcmd.New(stdout, stderr),
		Kitty: kittycmd.New(stdout, stderr),
		SSH:   sshcmd.New(stdout, stderr),
	}
}

type parserExit struct {
	code int
}

func Run(args []string, stdout, stderr io.Writer) (err error) {
	// Kong uses its configured Exit function while parsing help and while
	// running commands such as shell completion. Convert both cases into a
	// normal return instead of allowing the CLI library to terminate the process.
	defer func() {
		recovered := recover()
		if recovered == nil {
			return
		}

		exit, ok := recovered.(parserExit)
		if !ok {
			panic(recovered)
		}

		if exit.code == 0 {
			err = nil
			return
		}

		err = fmt.Errorf("CLI exited with code %d", exit.code)
	}()

	if len(args) == 0 && os.Getenv("COMP_LINE") == "" {
		_, err = parseArgs([]string{"--help"}, stdout, stderr)
		return
	}

	ctx, err := parseArgs(args, stdout, stderr)
	if err != nil {
		if parseErr, ok := errors.AsType[*kong.ParseError](err); ok {
			parseErr.Context.Stdout = stderr
			_ = parseErr.Context.PrintUsage(false)
		}

		return err
	}

	return ctx.Run()
}

func parseArgs(args []string, stdout, stderr io.Writer) (*kong.Context, error) {
	app := newRootCommand(stdout, stderr)

	parser, err := kong.New(
		app,
		kong.Name("cli-toolbox"),
		kong.Description("cli-toolbox helps with small workflow tasks."),
		kong.Writers(stdout, stderr),
		kong.Exit(func(code int) {
			panic(parserExit{code: code})
		}),
	)
	if err != nil {
		return nil, fmt.Errorf("build CLI parser: %w", err)
	}

	kongcompletion.Register(parser, worktreeNamesPredictor)

	return parser.Parse(args)
}
