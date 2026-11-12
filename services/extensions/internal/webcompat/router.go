// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package webcompat

import (
	"net/http"
	"strings"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/acquisition"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/checkins"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/community"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/herosms"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/marketreview"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/operations"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

// Mount keeps interactive Web traffic separate from the service-token module
// API while sharing its admission/drain lifecycle. It never upgrades the host
// credential into user authority.
type AdditionalServices struct {
	Checkins             *checkins.Service
	HeroSMS              *herosms.Service
	MarketReview         *marketreview.Engine
	Builtins             BuiltinHTTP
	ToolMarketMCP        http.Handler
	Assistant            AssistantHistory
	AssistantChat        AssistantChat
	AssistantServices    http.Handler
	ToolMarket           ToolMarketService
	Operations           *operations.Service
	Payments             PaymentStatusReader
	PaymentCheckout      PaymentCheckoutService
	PaymentPortal        PaymentPortal
	PancakeAdmin         PancakeAdmin
	SubscriptionPayments http.Handler
	Plugins              PluginController
	Community            *community.Service
	Acquisition          *acquisition.Service
}

// @mark extensions.admission role=consumer
func Mount(host *modules.Host, auth *Authenticator, catalogue StoreService, support SupportService, optional ...AdditionalServices) http.Handler {
	mux := http.NewServeMux()
	var additional AdditionalServices
	if len(optional) == 1 {
		additional = optional[0]
	}
	routes := routeRegistry{mux: mux, host: host, plugins: additional.Plugins}
	RegisterStore(routes.forPlugins("store"), auth, catalogue)
	RegisterSupport(routes.forPlugins("support"), auth, support)
	var operationsService *operations.Service
	var paymentReader PaymentStatusReader
	var paymentCheckout PaymentCheckoutService
	var paymentPortal PaymentPortal
	if len(optional) == 1 {
		RegisterHeroSMS(routes.forPlugins("hero-sms"), auth, additional.HeroSMS)
		RegisterMarketAIReview(routes.forPlugins("store", "toolmarket"), auth, additional.MarketReview)
		registerProductAIReviews(routes.forPlugins("store"), auth, catalogue)
		registerToolAIReviews(routes.forPlugins("toolmarket"), auth, additional.ToolMarket)
		RegisterBuiltinHTTP(routes.forPlugins("toolmarket"), additional.Builtins)
		if !store.Missing(additional.ToolMarketMCP) {
			routes.forPlugins("toolmarket").Handle("/mcp/market", additional.ToolMarketMCP)
			routes.forPlugins("toolmarket").Handle("/mcp/market/", additional.ToolMarketMCP)
		}
		operationsService = additional.Operations
		paymentReader = additional.Payments
		paymentCheckout = additional.PaymentCheckout
		paymentPortal = additional.PaymentPortal
		RegisterPaymentPortal(routes.forPlugins("payments"), auth, paymentPortal)
		RegisterPancakeAdmin(routes.forPlugins("payments"), auth, additional.PancakeAdmin)
		if !store.Missing(additional.SubscriptionPayments) {
			subscriptions := http.StripPrefix("/api/user/payments/subscriptions", additional.SubscriptionPayments)
			routes.forPlugins("payments").Handle("/api/user/payments/subscriptions/", http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if _, err := auth.Verify(r.Context(), r); err != nil {
					Respond(w, nil, err)
					return
				}
				request := r.Clone(r.Context())
				request.Header.Set("X-LMM-User-Credential", strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer "))
				subscriptions.ServeHTTP(w, request)
			}))
		}
		if !store.Missing(additional.Assistant) {
			RegisterAssistant(routes.forPlugins("assistant"), auth, additional.Assistant, additional.AssistantChat)
		}
		if !store.Missing(additional.AssistantServices) {
			handler := additional.AssistantServices
			if support, ok := handler.(AssistantSupport); ok {
				RegisterAssistantSupport(routes.forPlugins("assistant"), auth, support)
			}
			routes.forPlugins("assistant").Handle("/api/assistant/services/", http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if _, err := auth.Verify(r.Context(), r); err != nil {
					Respond(w, nil, err)
					return
				}
				handler.ServeHTTP(w, r)
			}))
		}
		if !store.Missing(additional.ToolMarket) {
			RegisterToolMarket(routes.forPlugins("toolmarket"), auth, additional.ToolMarket)
		}
		if !store.Missing(additional.Plugins) {
			RegisterPlugins(routes, auth, additional.Plugins)
		}
		if additional.Checkins != nil {
			additional.Checkins.Register(routes.forPlugins("checkins"))
		}
		RegisterCommunity(routes.forPlugins("community"), auth, additional.Community)
		RegisterAcquisition(routes.forPlugins("acquisition"), auth, additional.Acquisition)
	}
	RegisterOperations(routes.forPlugins("operations"), auth, operationsService)
	RegisterPayments(routes.forPlugins("payments"), auth, paymentReader)
	RegisterPaymentCheckout(routes.forPlugins("payments"), auth, paymentCheckout, paymentPortal)
	// Unassembled old routes are explicit failures, never a successful empty DTO.
	routes.forPlugins("store").HandleFunc("/api/store/", func(w http.ResponseWriter, r *http.Request) {
		if _, err := auth.Verify(r.Context(), r); err != nil {
			Respond(w, nil, err)
			return
		}
		Respond(w, nil, store.ErrUnavailable)
	})
	// Forward only the host's own namespaces so its fallback cannot swallow
	// method-not-allowed responses from registered Web routes.
	mux.Handle("/extensions/v1/", host)
	mux.Handle("/health/", host)
	return OperationsGuard(mux)
}
