// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"

	"github.com/expr-lang/expr"
	"github.com/expr-lang/expr/ast"
	"github.com/expr-lang/expr/conf"
	"github.com/expr-lang/expr/parser"
	"github.com/expr-lang/expr/vm"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// A program may bind read results before its final operation. A single final
// mutation keeps MCP confirmation, idempotency and native content attached to
// the exact invocation that produced them.
type marketCodeStep struct {
	bind   string
	action string
	args   *vm.Program
}

const marketCodeBytes = 8192

type marketCodeValues struct {
	bindings map[string]bool
	invalid  bool
}

func (v *marketCodeValues) Visit(n *ast.Node) {
	switch x := (*n).(type) {
	case *ast.IdentifierNode:
		if !v.bindings[x.Value] {
			v.invalid = true
		}
	case *ast.MemberNode:
		if x.Method {
			v.invalid = true
		}
	case *ast.UnaryNode:
		if x.Operator != "-" {
			v.invalid = true
		}
	case *ast.PairNode:
		if _, ok := x.Key.(*ast.StringNode); !ok {
			v.invalid = true
		}
	case *ast.MapNode, *ast.ArrayNode, *ast.StringNode, *ast.BoolNode, *ast.NilNode, *ast.IntegerNode, *ast.FloatNode:
	default:
		v.invalid = true
	}
}

func parseMarketCode(raw json.RawMessage) ([]marketCodeStep, error) {
	var input struct {
		Code string `json:"code"`
	}
	if decodeStrict(raw, &input) != nil || len(input.Code) == 0 || len(input.Code) > marketCodeBytes {
		return nil, fmt.Errorf("metamcp expects a single code string, for example search({query:'drawing',limit:1}): %w", ErrInvalid)
	}
	cfg := conf.CreateNew()
	cfg.MaxNodes = 256
	tree, err := parser.ParseWithConfig(input.Code, cfg)
	if err != nil {
		return nil, fmt.Errorf("invalid market expression: %w", ErrInvalid)
	}
	bindings := map[string]bool{}
	steps := []marketCodeStep{}
	node := tree.Node
	for len(steps) < 8 {
		bind := ""
		value := node
		if decl, ok := node.(*ast.VariableDeclaratorNode); ok {
			bind, value, node = decl.Name, decl.Value, decl.Expr
			if bindings[bind] {
				return nil, ErrInvalid
			}
		}
		call, ok := value.(*ast.CallNode)
		if !ok || len(call.Arguments) != 1 {
			return nil, ErrInvalid
		}
		name, ok := call.Callee.(*ast.IdentifierNode)
		if !ok {
			return nil, ErrInvalid
		}
		if _, ok = legacyMCPActions[name.Value]; !ok {
			return nil, ErrInvalid
		}
		if bind != "" {
			switch name.Value {
			case "search", "details", "status", "calls", "usage", "call_status":
			default:
				return nil, fmt.Errorf("only read operations can be bound with let: %w", ErrInvalid)
			}
		}
		args := call.Arguments[0]
		if _, ok = args.(*ast.MapNode); !ok {
			return nil, ErrInvalid
		}
		check := marketCodeValues{bindings: bindings}
		ast.Walk(&args, &check)
		if check.invalid {
			return nil, ErrInvalid
		}
		prog, err := expr.Compile(args.String(), expr.AllowUndefinedVariables(), expr.DisableAllBuiltins(), expr.MaxNodes(256), expr.Optimize(false))
		if err != nil {
			return nil, ErrInvalid
		}
		steps = append(steps, marketCodeStep{bind, name.Value, prog})
		if bind == "" {
			return steps, nil
		}
		bindings[bind] = true
	}
	return nil, ErrLimit
}

func marketActionScopes(action string) (invoke, manage bool) {
	switch action {
	case "invoke":
		return true, false
	case "authorize", "set_tool_budget", "set_client_budget":
		return true, true
	case "load", "unload":
		return false, true
	default:
		return false, false
	}
}

func (m *Module) runMarketCode(ctx context.Context, raw string, p LegacyMarketPrincipal, req *mcp.CallToolRequest) (*mcp.CallToolResult, error) {
	steps, err := parseMarketCode(req.Params.Arguments)
	if err != nil {
		return legacyMCPOutput(nil, err)
	}
	if (req.Params.RequestState != "" || len(req.Params.InputResponses) > 0) && steps[len(steps)-1].action != "invoke" {
		return legacyMCPOutput(nil, ErrInvalid)
	}
	vars := map[string]any{}
	for _, step := range steps {
		invoke, manage := marketActionScopes(step.action)
		if invoke && !p.CanInvoke || manage && !p.CanManage {
			return legacyMCPOutput(nil, ErrLegacyMarketScope)
		}
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		args, err := expr.Run(step.args, vars)
		fields, ok := args.(map[string]any)
		if err != nil || !ok {
			return legacyMCPOutput(nil, ErrInvalid)
		}
		if _, exists := fields["action"]; exists {
			return legacyMCPOutput(nil, ErrInvalid)
		}
		fields["action"] = step.action
		encoded, err := json.Marshal(fields)
		if err != nil {
			return legacyMCPOutput(nil, ErrInvalid)
		}
		in, err := legacyDecodeMCPInput(encoded, m.now().Unix())
		if err != nil {
			return legacyMCPOutput(nil, err)
		}
		if in.Action == "invoke" {
			return m.legacyMCPInvoke(ctx, raw, p, in, req)
		}
		result, err := m.legacyMCPMeta(ctx, raw, p, in)
		if view, ok := result.(map[string]any); ok {
			if _, found := view["refresh_tools_list"]; found {
				view["refresh_tools_list"] = false
			}
		}
		if err != nil || step.bind == "" {
			return legacyMCPOutput(result, err)
		}
		data, err := json.Marshal(result)
		if err != nil || len(data) > 128<<10 {
			return legacyMCPOutput(nil, ErrLimit)
		}
		// Only JSON enters the expression environment; do not expose Go methods or
		// round IDs, amounts or tool arguments through floating point.
		dec := json.NewDecoder(bytes.NewReader(data))
		dec.UseNumber()
		var jsonValue any
		if dec.Decode(&jsonValue) != nil {
			return legacyMCPOutput(nil, ErrInvalid)
		}
		vars[step.bind] = jsonValue
	}
	return legacyMCPOutput(nil, ErrInvalid)
}
