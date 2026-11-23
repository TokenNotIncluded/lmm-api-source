package app

import (
	"context"
	"errors"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant_services"
)

func validateAssistantSearchConfiguration(cfg GoConfig) error {
	search := cfg.Assistant.Search
	switch search.Provider {
	case "", "none":
		if search.Endpoint != "" || search.APIKeyFile != "" || search.TimeoutMS != 0 {
			return errors.New("disabled assistant search cannot have provider settings")
		}
		return nil
	case "exa":
	default:
		return errors.New("assistant search provider must be exa or none")
	}
	if !cfg.Assistant.AgentRequired {
		return errors.New("assistant search requires the installed agent")
	}
	if !absolutePath(search.APIKeyFile) {
		return errors.New("assistant search requires an absolute private credential file")
	}
	if search.TimeoutMS < 0 || search.TimeoutMS > 30000 {
		return errors.New("assistant search timeout is out of bounds")
	}
	return nil
}

// A missing provider stays nil so AvailableBindings does not advertise search.
// Only this host-selected dependency carries the private provider credential;
// model arguments still pass through the existing scoped durable search tool.
func assistantServiceDependencies(cfg GoConfig) (func(context.Context, *coreclient.AssistantClient) (assistant_services.Dependencies, error), error) {
	mail, err := assistantInvitationSender(cfg)
	if err != nil {
		return nil, err
	}
	if cfg.Assistant.Search.Provider == "" || cfg.Assistant.Search.Provider == "none" {
		if mail == nil {
			return nil, nil
		}
		return func(context.Context, *coreclient.AssistantClient) (assistant_services.Dependencies, error) {
			return assistant_services.Dependencies{InvitationMail: mail}, nil
		}, nil
	}
	raw, err := coreclient.ReadPrivateFile(cfg.Assistant.Search.APIKeyFile, 4096)
	if err != nil {
		return nil, err
	}
	provider, err := assistant_services.NewSearchProvider(assistant_services.SearchConfig{
		Provider:  cfg.Assistant.Search.Provider,
		Endpoint:  cfg.Assistant.Search.Endpoint,
		APIKey:    string(raw),
		LocalHTTP: cfg.Host.LocalHTTP,
		Timeout:   time.Duration(cfg.Assistant.Search.TimeoutMS) * time.Millisecond,
	})
	if err != nil {
		return nil, err
	}
	return func(context.Context, *coreclient.AssistantClient) (assistant_services.Dependencies, error) {
		return assistant_services.Dependencies{Search: provider, InvitationMail: mail}, nil
	}, nil
}
