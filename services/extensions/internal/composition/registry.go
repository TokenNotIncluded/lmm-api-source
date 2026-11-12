// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package composition

import (
	"fmt"
	"slices"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/identity"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/remotecontrol"
)

type moduleRegistration struct {
	descriptor modules.Descriptor
	bind       func(*Runtime) (modules.Module, error)
}

// This is the compiled package catalogue. Selection, lifecycle metadata and
// module binding all read these registrations; storage and Core authority are
// still assembled explicitly by New before a module is exposed.
var moduleRegistrations = []moduleRegistration{
	{
		descriptor: modules.Descriptor{Name: "identity", Version: "1.0.0"},
		bind:       func(r *Runtime) (modules.Module, error) { return identity.New(r.Client), nil },
	},
	{
		descriptor: modules.Descriptor{Name: "store", Version: "1.0.0", NavigationPaths: []string{"/store"}},
		bind:       func(r *Runtime) (modules.Module, error) { return r.Store, nil },
	},
	{
		descriptor: modules.Descriptor{Name: "support", Version: "1.0.0", Requires: []string{"store"}, NavigationPaths: []string{"/support", "/store/support"}},
		bind:       func(r *Runtime) (modules.Module, error) { return r.Support, nil },
	},
	{
		descriptor: modules.Descriptor{Name: "assistant", Version: "1.0.0", NavigationPaths: []string{"/chat-management"}},
		bind:       func(r *Runtime) (modules.Module, error) { return r.Assistant, nil },
	},
	{
		descriptor: modules.Descriptor{Name: "remote-control", Version: "1.0.0", NavigationPaths: []string{"/remote-control"}},
		bind:       func(r *Runtime) (modules.Module, error) { return remotecontrol.New(r.Client) },
	},
	{
		descriptor: modules.Descriptor{Name: "payments", Version: "1.0.0"},
		bind: func(r *Runtime) (modules.Module, error) {
			return paymentModule{wallet: r.Payments, subscriptions: r.SubscriptionPayments, stripe: r.StripeSubscriptionPayments, pancake: r.PancakeSubscriptionPayments}, nil
		},
	},
	{
		descriptor: modules.Descriptor{Name: "toolmarket", Version: "1.0.0", NavigationPaths: []string{"/tool-market"}},
		bind:       func(r *Runtime) (modules.Module, error) { return r.ToolMarket, nil },
	},
	{
		descriptor: modules.Descriptor{Name: "hero-sms", Version: "1.0.0", NavigationPaths: []string{"/temporary-activations", "/email-activations"}},
		bind:       func(r *Runtime) (modules.Module, error) { return r.HeroSMS, nil },
	},
	// The process entry supplies the authenticated Web adapters for these
	// services after loading the host credential and constructing its verifier.
	{descriptor: modules.Descriptor{Name: "operations", Version: "1.0.0", NavigationPaths: []string{"/rss", "/scripts", "/operations/releases"}}},
	{descriptor: modules.Descriptor{Name: "community", Version: "1.0.0", NavigationPaths: []string{"/ai-directory", "/rankings", "/games/signal"}}},
	{descriptor: modules.Descriptor{Name: "acquisition", Version: "1.0.0", NavigationPaths: []string{"/operations/sources"}}},
}

func ModuleDescriptors() []modules.Descriptor {
	result := make([]modules.Descriptor, len(moduleRegistrations))
	for i, registration := range moduleRegistrations {
		result[i] = registration.descriptor
		result[i].Requires = slices.Clone(result[i].Requires)
		result[i].NavigationPaths = slices.Clone(result[i].NavigationPaths)
	}
	return result
}

// SelectModules requires dependencies to be explicitly selected. Registering a
// new package never enables it or grants access to another package's resources.
func SelectModules(config string, defaults []string) ([]string, error) {
	names := make([]string, len(moduleRegistrations))
	for i, registration := range moduleRegistrations {
		names[i] = registration.descriptor.Name
	}
	selected, err := modules.Select(config, defaults, names)
	if err != nil {
		return nil, err
	}
	for _, registration := range moduleRegistrations {
		if !slices.Contains(selected, registration.descriptor.Name) {
			continue
		}
		for _, required := range registration.descriptor.Requires {
			if !slices.Contains(selected, required) {
				return nil, fmt.Errorf("%s requires the %s module", registration.descriptor.Name, required)
			}
		}
	}
	return selected, nil
}

func (r *Runtime) bindModules(selected []string) error {
	for _, name := range selected {
		for _, registration := range moduleRegistrations {
			if registration.descriptor.Name != name || registration.bind == nil {
				continue
			}
			module, err := registration.bind(r)
			if err != nil {
				return err
			}
			r.Modules = append(r.Modules, module)
			break
		}
	}
	return nil
}
