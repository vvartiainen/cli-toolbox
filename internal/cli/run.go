package cli

import (
	"errors"
	"fmt"
	"io"

	"cli-toolbox/internal/cli/awscmd"
	"cli-toolbox/internal/cli/kittycmd"
	"cli-toolbox/internal/cli/sshcmd"

	"github.com/alecthomas/kong"
)

type rootCommand struct {
	AWS   awscmd.Command   `cmd:"" name:"aws" help:"AWS helpers."`
	Kitty kittycmd.Command `cmd:"" name:"kitty" help:"Kitty helpers."`
	SSH   sshcmd.Command   `cmd:"" name:"ssh" help:"SSH helpers."`
}

func newRootCommand(stdout, stderr io.Writer) *rootCommand {
	return &rootCommand{
		AWS:   awscmd.New(stdout, stderr),
		Kitty: kittycmd.New(stdout, stderr),
		SSH:   sshcmd.New(stdout, stderr),
	}
}

type parserExit struct {
	code int
}

func Run(args []string, stdout, stderr io.Writer) (err error) {
	if len(args) == 0 {
		_, _, err = parseArgs([]string{"--help"}, stdout, stderr)
		return err
	}

	ctx, exited, err := parseArgs(args, stdout, stderr)
	if exited != nil {
		if exited.code == 0 {
			return nil
		}

		return fmt.Errorf("CLI exited with code %d", exited.code)
	}

	if err != nil {
		if parseErr, ok := errors.AsType[*kong.ParseError](err); ok {
			parseErr.Context.Stdout = stderr
			_ = parseErr.Context.PrintUsage(false)
		}

		return err
	}

	return ctx.Run()
}

func parseArgs(args []string, stdout, stderr io.Writer) (ctx *kong.Context, exited *parserExit, err error) {
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
		return nil, nil, fmt.Errorf("build CLI parser: %w", err)
	}

	defer func() {
		recovered := recover()
		if recovered == nil {
			return
		}

		exit, ok := recovered.(parserExit)
		if !ok {
			panic(recovered)
		}

		exited = &exit
		ctx = nil
		err = nil
	}()

	ctx, err = parser.Parse(args)
	return ctx, exited, err
}
