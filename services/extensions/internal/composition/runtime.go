package composition

import (
	"context"
	"errors"
	"net/http"
	"slices"
	"sync"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/acquisition"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant_services"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/checkins"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/community"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/herosms"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/marketreview"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/operations"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/payments"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store/rustauth"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store/rustfunds"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/support"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

type Config struct {
	Checkins                      StorageConfig
	CheckinTokenFile              string
	HeroSMSSocket                 string
	HeroSMSTokenFile              string
	MarketReviewSocket            string
	MarketReviewTokenFile         string
	MarketReviewSettingsFile      string
	ToolMarketCorePublicOrigin    string
	LocalHTTP                     bool
	HostListen                    string
	HostPublicOrigin              string
	ToolMarketWitnessSocket       string
	ToolMarketWitnessTokenFile    string
	ToolMarketBuiltins            ToolMarketBuiltinConfig
	ToolMarketMeteringFile        string
	SupportCustomerSocket         string
	SupportCustomerTokenFile      string
	SupportNoteKeyFile            string
	ToolMarket                    StorageConfig
	ToolMarketKeyFile             string
	ToolMarketCallbackURL         string
	ToolMarketSocket              string
	ToolMarketTokenFile           string
	HostCredential                []byte
	Selected                      []string
	Socket                        string
	TokenFile                     string
	PaymentTokenFile              string
	PaymentConfigFile             string
	Payments                      StorageConfig
	CommerceTokenFile             string
	DirectoryAdTokenFile          string
	Store                         StorageConfig
	StoreStockKeyFile             string
	StoreExtoreKeyFile            string
	StoreExtoreRedirectURI        string
	Support                       StorageConfig
	Assistant                     StorageConfig
	AssistantCursorKeyFile        string
	AssistantTokenFile            string
	AssistantAgentRequired        bool
	AssistantServicesDependencies func(context.Context, *coreclient.AssistantClient) (assistant_services.Dependencies, error)
	AssistantServicesPolicies     assistant_services.PolicyReader
	AssistantPublicSnapshot       assistant_services.PublicSnapshotReader
	Operations                    StorageConfig
	Community                     StorageConfig
	Acquisition                   StorageConfig
	UserReadTokenFile             string
	UserReadSocket                string
	Scripts                       *operations.ScriptConfig
	ScriptRepositoryKeyFile       string
	IoNetKeyFile                  string
}

type Runtime struct {
	Checkins                    *checkins.Service
	HeroSMS                     *herosms.Service
	MarketReview                *marketreview.Engine
	paymentIsolationCredentials [][]byte
	Builtins                    *toolmarket.BuiltinDispatcher
	ToolMarketMCP               http.Handler
	ToolMarket                  *toolmarket.Module
	Modules                     []modules.Module
	Client                      *coreclient.Client
	PaymentClient               *coreclient.Client
	Payments                    *payments.Service
	SubscriptionPayments        *payments.EPaySubscriptions
	StripeSubscriptionPayments  *payments.StripeSubscriptions
	PancakeSubscriptionPayments *payments.PancakeSubscriptions
	PancakeAdmin                *PancakeAdministration
	CommerceClient              *coreclient.Client
	Store                       *store.Service
	Support                     *support.Service
	Assistant                   *assistant.Service
	AssistantHistory            *assistant.HistoryService
	AssistantModelClient        *coreclient.AssistantClient
	AssistantPending            *assistant.PendingConversations
	AssistantTurns              *assistant.DelegatedTurns
	AssistantChat               assistant.ChatPort
	AssistantServices           *assistant_services.Service
	Operations                  *operations.Service
	Community                   *community.Service
	Acquisition                 *acquisition.Service
	close                       []func() error
	workerCancel                context.CancelFunc
	workerDone                  chan struct{}
	workerOnce                  sync.Once
}

func (r *Runtime) Close() error {
	if r.workerCancel != nil {
		r.workerCancel()
		<-r.workerDone
		r.workerCancel = nil
	}
	var err error
	for i := len(r.close) - 1; i >= 0; i-- {
		err = errors.Join(err, r.close[i]())
	}
	r.close = nil
	return err
}

// StartWorkers is explicit and runs after listener readiness. One bounded
// recovery batch shares host admission and is joined before closing resources.
// @mark extensions.admission role=consumer
// @mark extensions.recovery role=scheduler
func (r *Runtime) StartWorkers(parent context.Context, host *modules.Host) {
	r.workerOnce.Do(func() {
		ctx, cancel := context.WithCancel(parent)
		r.workerCancel = cancel
		r.workerDone = make(chan struct{})
		go func() {
			defer close(r.workerDone)
			ticker := time.NewTicker(5 * time.Second)
			defer ticker.Stop()
			for {
				select {
				case <-ctx.Done():
					return
				case <-ticker.C:
				}
				var recovery []func(context.Context) error
				if r.Payments != nil {
					recovery = append(recovery, func(work context.Context) error { return r.Payments.Work(work, 1) })
				}
				if r.SubscriptionPayments != nil {
					recovery = append(recovery, func(work context.Context) error { return r.SubscriptionPayments.Recover(work, 1) })
				}
				if r.PancakeSubscriptionPayments != nil {
					recovery = append(recovery, func(work context.Context) error { return r.PancakeSubscriptionPayments.Recover(work, 1) })
				}
				if r.StripeSubscriptionPayments != nil {
					recovery = append(recovery, func(work context.Context) error { return r.StripeSubscriptionPayments.Recover(work, 1) })
				}
				if r.Store != nil {
					recovery = append(recovery, r.Store.RecoverDeliveries)
					if r.MarketReview != nil {
						recovery = append(recovery, func(work context.Context) error { return r.Store.WorkAIReviews(work, 1) })
					}
				}
				if r.ToolMarket != nil {
					recovery = append(recovery, func(work context.Context) error { _, err := r.ToolMarket.RecoverSettlements(work, 1); return err }, func(work context.Context) error { _, err := r.ToolMarket.LegacyRecoverPending(work, 1); return err })
					if r.MarketReview != nil {
						recovery = append(recovery, func(work context.Context) error { return r.ToolMarket.WorkAIReviews(work, 1) })
					}
				}
				for _, recover := range recovery {
					if ctx.Err() != nil {
						return
					}
					release, err := host.TryAcquire()
					if errors.Is(err, modules.ErrSaturated) {
						// Retry only on the next polling tick, without queueing or spinning.
						break
					}
					if err != nil {
						return
					}
					bounded, stop := context.WithTimeout(ctx, 20*time.Second)
					_ = recover(bounded)
					stop()
					release()
				}
			}
		}()
	})
}

func New(ctx context.Context, cfg Config) (_ *Runtime, resultErr error) {
	selection, err := SelectModules("", cfg.Selected)
	if err != nil {
		return nil, err
	}
	cfg.Selected = selection
	if err := validateHeroSMSSelection(cfg); err != nil {
		return nil, err
	}
	if err := validateMarketReviewSelection(cfg); err != nil {
		return nil, err
	}
	if err := ValidateHostProfile(cfg.LocalHTTP, cfg.HostListen, cfg.HostPublicOrigin); err != nil {
		return nil, err
	}
	if err := validateSupportCustomersSelection(cfg); err != nil {
		return nil, err
	}
	if err := validateToolMarketSelection(cfg); err != nil {
		return nil, err
	}
	if err := validateAssistantSelection(cfg); err != nil {
		return nil, err
	}
	if err := validatePaymentSelection(cfg); err != nil {
		return nil, err
	}
	if (cfg.UserReadTokenFile != "" || cfg.UserReadSocket != "") && !slices.Contains(cfg.Selected, "community") && !slices.Contains(cfg.Selected, "acquisition") {
		return nil, errors.New("Core read configuration requires community or acquisition")
	}
	if (cfg.UserReadSocket == "") != (cfg.UserReadTokenFile == "") {
		return nil, errors.New("configure both Core read socket and token file")
	}
	if slices.Contains(cfg.Selected, "checkins") != (cfg.CheckinTokenFile != "") || (!slices.Contains(cfg.Selected, "checkins") && (cfg.Checkins.URLFile != "" || cfg.Checkins.Database != "")) {
		return nil, errors.New("checkins requires explicit selection and a dedicated funding credential")
	}
	r := &Runtime{}
	defer func() {
		if resultErr != nil {
			_ = r.Close()
		}
	}()
	if len(cfg.Selected) == 0 {
		if cfg.Operations.URLFile != "" || cfg.Operations.Database != "" || cfg.Scripts != nil || cfg.ScriptRepositoryKeyFile != "" || cfg.IoNetKeyFile != "" {
			return nil, errors.New("operations configuration requires explicit module selection")
		}
		return r, nil
	}
	if cfg.Socket == "" || cfg.TokenFile == "" {
		return nil, errors.New("selected modules require core RPC configuration")
	}
	token, err := coreclient.ReadTokenFile(cfg.TokenFile)
	if err != nil {
		return nil, err
	}
	r.Client, err = coreclient.New(cfg.Socket, token)
	if err != nil {
		return nil, err
	}
	r.close = append(r.close, r.Client.Close)
	selected := map[string]bool{}
	for _, name := range cfg.Selected {
		selected[name] = true
	}
	if cfg.CommerceTokenFile != "" && !selected["store"] {
		return nil, errors.New("commerce RPC configuration requires store")
	}
	var commerceToken []byte
	if cfg.CommerceTokenFile != "" {
		commerceToken, err = coreclient.ReadTokenFile(cfg.CommerceTokenFile)
		if err != nil {
			return nil, err
		}
	}
	if err := independentRuntimeCredentials(cfg.HostCredential, token, commerceToken); err != nil {
		return nil, err
	}
	var paymentToken []byte
	if selected["payments"] {
		paymentToken, err = loadPaymentCredential(cfg, token, commerceToken)
		if err != nil {
			return nil, err
		}
	}
	assistantToken, err := loadAssistantCredential(cfg, token, commerceToken, paymentToken)
	if err != nil {
		return nil, err
	}
	supportCustomerToken, supportNoteKey, err := loadSupportCustomerCredentials(cfg, token, commerceToken, paymentToken, assistantToken)
	if err != nil {
		return nil, err
	}
	if err := r.composeHeroSMS(ctx, cfg, token, commerceToken, paymentToken, assistantToken, supportCustomerToken, supportNoteKey); err != nil {
		return nil, err
	}
	if selected["toolmarket"] {
		if err := r.composeToolMarket(ctx, cfg, token, commerceToken, paymentToken, assistantToken, supportCustomerToken, supportNoteKey); err != nil {
			return nil, err
		}
	}
	if selected["payments"] {
		if err := r.composePayments(ctx, cfg, paymentToken, token, commerceToken, assistantToken, supportCustomerToken, supportNoteKey); err != nil {
			return nil, err
		}
	}
	if selected["checkins"] {
		fundingToken, e := coreclient.ReadTokenFile(cfg.CheckinTokenFile)
		if e != nil {
			return nil, e
		}
		for _, path := range []string{cfg.DirectoryAdTokenFile, cfg.HeroSMSTokenFile, cfg.MarketReviewTokenFile, cfg.ToolMarketTokenFile, cfg.ToolMarketWitnessTokenFile} {
			if path != "" {
				other, e := coreclient.ReadTokenFile(path)
				if e != nil {
					return nil, e
				}
				if e = independentRuntimeCredentials(fundingToken, other); e != nil {
					return nil, e
				}
			}
		}
		if e = independentRuntimeCredentials(cfg.HostCredential, token, commerceToken, paymentToken, assistantToken, supportCustomerToken, supportNoteKey, fundingToken); e != nil {
			return nil, e
		}
		client, e := coreclient.New(cfg.Socket, fundingToken)
		if e != nil {
			return nil, e
		}
		r.close = append(r.close, client.Close)
		db, e := openStorage(ctx, cfg.Checkins)
		if e != nil {
			return nil, e
		}
		r.close = append(r.close, db.Close)
		r.Checkins, e = checkins.New(ctx, db, coreclient.CheckinLedger{Client: client})
		if e != nil {
			return nil, e
		}
	}
	if selected["store"] {
		db, err := openStorage(ctx, cfg.Store)
		if err != nil {
			return nil, err
		}
		r.close = append(r.close, db.Close)
		repo, err := store.NewPostgres(db)
		if err != nil {
			return nil, err
		}
		auth, err := rustauth.New(r.Client)
		if err != nil {
			return nil, err
		}
		// A real commerce funds adapter is deliberately required before money
		// operations can become usable. Identity does not grant spending.
		var funds store.Funds = store.DisabledFunds{}
		if cfg.CommerceTokenFile != "" {
			r.CommerceClient, err = coreclient.New(cfg.Socket, commerceToken)
			if err != nil {
				return nil, err
			}
			r.close = append(r.close, r.CommerceClient.Close)
			caps, err := r.Client.Capabilities(ctx)
			if err != nil || caps == nil {
				return nil, errors.New("commerce core capabilities unavailable")
			}
			funds, err = rustfunds.New(r.CommerceClient, caps.GetFeatures())
			if err != nil {
				return nil, errors.New("commerce core capabilities unavailable")
			}
		}
		r.Store, err = store.New(repo, auth, funds)
		if err != nil {
			return nil, err
		}
		if cfg.StoreStockKeyFile != "" {
			key, keyErr := coreclient.ReadPrivateFile(cfg.StoreStockKeyFile, 32)
			if keyErr != nil || len(key) != 32 {
				return nil, errors.New("store stock key must contain exactly 32 bytes")
			}
			if err := independentRuntimeCredentials(key, cfg.HostCredential, token, commerceToken, paymentToken, assistantToken, supportCustomerToken, supportNoteKey); err != nil {
				return nil, err
			}
			if err := r.Store.WithStockKey(key); err != nil {
				return nil, err
			}
		}
		if cfg.StoreExtoreKeyFile != "" {
			key, keyErr := coreclient.ReadPrivateFile(cfg.StoreExtoreKeyFile, 32)
			if keyErr != nil || len(key) != 32 {
				return nil, errors.New("store Extore key must contain exactly 32 bytes")
			}
			if err := independentRuntimeCredentials(key, cfg.HostCredential, token, commerceToken, paymentToken, assistantToken, supportCustomerToken, supportNoteKey); err != nil {
				return nil, err
			}
			if err := r.Store.WithExtore(cfg.StoreExtoreRedirectURI, key); err != nil {
				return nil, err
			}
		}
		if selected["support"] {
			db, err := openStorage(ctx, cfg.Support)
			if err != nil {
				return nil, err
			}
			r.close = append(r.close, db.Close)
			repo, err := support.NewPostgres(db)
			if err != nil {
				return nil, err
			}
			r.Support, err = support.New(repo, auth, r.Store)
			if err != nil {
				return nil, err
			}
			if err := r.composeSupportCustomers(ctx, cfg, supportCustomerToken, supportNoteKey); err != nil {
				return nil, err
			}
		}
	}
	if err := r.composeMarketReview(ctx, cfg, token, commerceToken, paymentToken, assistantToken, supportCustomerToken, supportNoteKey); err != nil {
		return nil, err
	}
	if selected["assistant"] {
		db, err := openStorageBundle(ctx, cfg.Assistant, assistantStorageBundle(cfg))
		if err != nil {
			return nil, err
		}
		r.close = append(r.close, db.Close)
		repo, err := assistant.NewPostgres(ctx, db, cfg.Assistant.Database)
		if err != nil {
			return nil, err
		}
		auth, err := access.New(r.Client)
		if err != nil {
			return nil, err
		}
		key, err := coreclient.ReadTokenFile(cfg.AssistantCursorKeyFile)
		if err != nil {
			return nil, errors.New("assistant private cursor key unavailable")
		}
		if err := independentRuntimeCredentials(key, supportCustomerToken, supportNoteKey); err != nil {
			return nil, err
		}
		r.AssistantHistory, err = assistant.NewHistory(repo, auth, key)
		if err != nil {
			return nil, err
		}
		r.Assistant, err = assistant.New(repo, auth, nil, assistant.Options{})
		if err != nil {
			return nil, err
		}
		if err := r.composeAssistantDelegation(ctx, cfg, db, auth, assistantToken); err != nil {
			return nil, err
		}
	}
	if selected["operations"] {
		config := operations.Config{Scripts: cfg.Scripts}
		if cfg.Operations.URLFile != "" || cfg.Operations.Database != "" {
			db, err := openStorage(ctx, cfg.Operations)
			if err != nil {
				return nil, err
			}
			r.close = append(r.close, db.Close)
			config.Releases = db
		}
		if config.Releases == nil && config.Scripts == nil {
			return nil, errors.New("operations requires configured releases or reviewed scripts")
		}
		r.Operations, err = operations.New(ctx, config)
		if err != nil {
			return nil, err
		}
		if cfg.ScriptRepositoryKeyFile != "" {
			key, err := scriptRepositoryKey(cfg, cfg.HostCredential, token, commerceToken, paymentToken, assistantToken, supportCustomerToken, supportNoteKey)
			if err != nil {
				return nil, err
			}
			if err := r.Operations.Repository.WithCredentialKey(ctx, key); err != nil {
				return nil, err
			}
		}
		if cfg.IoNetKeyFile != "" {
			key, err := ioNetKey(cfg, cfg.HostCredential, token, commerceToken, paymentToken, assistantToken, supportCustomerToken, supportNoteKey)
			if err != nil {
				return nil, err
			}
			if err := r.Operations.IoNet.WithCredentialKey(ctx, key); err != nil {
				return nil, err
			}
		}

	} else if cfg.Operations.URLFile != "" || cfg.Operations.Database != "" || cfg.Scripts != nil || cfg.ScriptRepositoryKeyFile != "" || cfg.IoNetKeyFile != "" {
		return nil, errors.New("operations configuration requires explicit module selection")
	}
	var adCore community.DirectoryAdCore
	if cfg.DirectoryAdTokenFile != "" {
		if !selected["community"] {
			return nil, errors.New("advertising RPC configuration requires community")
		}
		adToken, e := coreclient.ReadTokenFile(cfg.DirectoryAdTokenFile)
		if e != nil {
			return nil, e
		}
		if e = independentRuntimeCredentials(cfg.HostCredential, token, commerceToken, paymentToken, assistantToken, adToken); e != nil {
			return nil, e
		}
		client, e := coreclient.New(cfg.Socket, adToken)
		if e != nil {
			return nil, e
		}
		r.close = append(r.close, client.Close)
		adCore = client
	}
	var reader *community.CoreRead
	if cfg.UserReadTokenFile != "" {
		token, e := coreclient.ReadTokenFile(cfg.UserReadTokenFile)
		if e != nil {
			return nil, e
		}
		reader, e = community.NewCoreRead(cfg.UserReadSocket, token)
		if e != nil {
			return nil, e
		}
		r.close = append(r.close, func() error { reader.Close(); return nil })
		if selected["community"] {
			if e = reader.ReadyUsage(ctx); e != nil {
				return nil, e
			}
		}
		if selected["acquisition"] {
			if e = reader.ReadyUsers(ctx); e != nil {
				return nil, e
			}
		}
	}
	if selected["community"] {
		db, err := openStorage(ctx, cfg.Community)
		if err != nil {
			return nil, err
		}
		r.close = append(r.close, db.Close)
		if r.Community, err = community.New(ctx, db, cfg.HostPublicOrigin, community.Options{Core: reader, AdCore: adCore}); err != nil {
			return nil, err
		}
	} else if cfg.Community.URLFile != "" || cfg.Community.Database != "" {
		return nil, errors.New("community configuration requires explicit module selection")
	}
	if selected["acquisition"] {
		db, err := openStorage(ctx, cfg.Acquisition)
		if err != nil {
			return nil, err
		}
		r.close = append(r.close, db.Close)
		if r.Acquisition, err = acquisition.New(ctx, db, cfg.HostPublicOrigin, !cfg.LocalHTTP); err != nil {
			return nil, err
		}

		if r.Community != nil {
			r.Acquisition.ClientRead = r.Community
		}
		if reader != nil {
			r.Acquisition.UserRead = reader
		}
	} else if cfg.Acquisition.URLFile != "" || cfg.Acquisition.Database != "" {
		return nil, errors.New("acquisition configuration requires explicit module selection")
	}
	if err := r.bindModules(cfg.Selected); err != nil {
		return nil, err
	}
	return r, nil
}
