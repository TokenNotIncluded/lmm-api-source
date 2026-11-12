package app

import (
	"bytes"
	"errors"
	"io"
	"net"
	"os"
	"path/filepath"
	"strings"
	"unicode/utf8"

	"github.com/BurntSushi/toml"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules"
)

const defaultConfigPath = "/etc/lmm.conf.d/extensions.toml"

type GoConfig struct {
	HeroSMS          HeroSMSConfig          `toml:"hero_sms"`
	MarketReview     MarketReviewConfig     `toml:"market_review"`
	SupportCustomers SupportCustomersConfig `toml:"support_customers"`
	ToolMarket       ToolMarketConfig       `toml:"toolmarket"`
	Version          int                    `toml:"version"`
	Host             HostConfig             `toml:"host"`
	Core             CoreConfig             `toml:"core"`
	Store            StoreConfig            `toml:"store"`
	Support          DatabaseConfig         `toml:"support"`
	Legal            LegalConfig            `toml:"legal"`
	Assistant        AssistantConfig        `toml:"assistant"`
	Operations       DatabaseConfig         `toml:"operations"`
	Community        DatabaseConfig         `toml:"community"`
	Acquisition      DatabaseConfig         `toml:"acquisition"`
	Payments         PaymentsConfig         `toml:"payments"`
	Scripts          ScriptConfig           `toml:"scripts"`
	IoNet            IoNetConfig            `toml:"ionet"`
}
type HostConfig struct {
	LocalHTTP       bool     `toml:"local_http"`
	PublicOrigin    string   `toml:"public_origin"`
	MaxInFlight     int      `toml:"max_in_flight"`
	Listen          string   `toml:"listen"`
	TokenFile       string   `toml:"token_file"`
	PluginStateFile string   `toml:"plugin_state_file"`
	Modules         []string `toml:"modules"`
	WebCompat       bool     `toml:"web_compat"`
}
type CoreConfig struct {
	UserReadTokenFile    string `toml:"user_read_token_file"`
	UserReadSocket       string `toml:"user_read_socket"`
	Socket               string `toml:"socket"`
	IdentityTokenFile    string `toml:"identity_token_file"`
	CommerceTokenFile    string `toml:"commerce_token_file"`
	DirectoryAdTokenFile string `toml:"directory_ad_token_file"`
	PaymentTokenFile     string `toml:"payment_token_file"`
	AssistantTokenFile   string `toml:"assistant_token_file"`
}
type DatabaseConfig struct {
	DatabaseName    string `toml:"database_name"`
	DatabaseURLFile string `toml:"database_url_file"`
}

// StoreConfig is the shop storage pool plus optional dedicated seal keys.
// stock_key_file and extore_key_file are private raw 32-byte files, never shared
// with Core tokens or each other.
type StoreConfig struct {
	DatabaseName      string `toml:"database_name"`
	DatabaseURLFile   string `toml:"database_url_file"`
	StockKeyFile      string `toml:"stock_key_file"`
	ExtoreKeyFile     string `toml:"extore_key_file"`
	ExtoreRedirectURI string `toml:"extore_redirect_uri"`
}
type LegalConfig struct {
	ConfigFile string `toml:"config_file"`
}
type AssistantConfig struct {
	DatabaseName    string `toml:"database_name"`
	DatabaseURLFile string `toml:"database_url_file"`
	CursorKeyFile   string `toml:"cursor_key_file"`
	AgentRequired   bool   `toml:"agent_required"`
}

type IoNetConfig struct {
	KeyFile string `toml:"key_file"`
}
type ScriptConfig struct {
	RepositoryKeyFile string           `toml:"repository_key_file"`
	Root              string           `toml:"root"`
	Manifest          []ReviewedScript `toml:"manifest"`
}
type ReviewedScript struct {
	Name   string `toml:"name"`
	SHA256 string `toml:"sha256"`
}

func configPath(args []string) (string, error) {
	if len(args) == 1 && args[0] == "--legacy-env" {
		return "", nil
	}
	if len(args) == 0 {
		return defaultConfigPath, nil
	}
	if len(args) == 2 && args[0] == "--config" && absolutePath(args[1]) {
		return args[1], nil
	}
	return "", errors.New("lmm-extensions accepts no subcommands; use --config with one absolute TOML file path or explicit --legacy-env")
}

func legacyConfig(getenv func(string) string) (GoConfig, error) {
	if err := validateDatabaseEnvironment(getenv); err != nil {
		return GoConfig{}, err
	}
	web := getenv("LMM_EXTENSION_WEB_COMPAT")
	if web != "" && web != "true" && web != "false" {
		return GoConfig{}, errors.New("invalid legacy Web compatibility option")
	}
	var selected []string
	if raw := getenv("LMM_EXTENSION_MODULES"); raw != "" {
		selected = strings.Split(raw, ",")
	}
	return GoConfig{Version: 1, Host: HostConfig{MaxInFlight: modules.DefaultMaxInFlight, Listen: getenv("LMM_EXTENSION_LISTEN"), TokenFile: getenv("LMM_EXTENSION_TOKEN_FILE"), PluginStateFile: getenv("LMM_EXTENSION_PLUGIN_STATE_FILE"), Modules: selected, WebCompat: web == "true"}, Core: CoreConfig{Socket: getenv("LMM_CORE_RPC_SOCKET"), IdentityTokenFile: getenv("LMM_CORE_RPC_TOKEN_FILE"), CommerceTokenFile: getenv("LMM_CORE_COMMERCE_RPC_TOKEN_FILE")}, Store: StoreConfig{DatabaseName: getenv("LMM_STORE_DATABASE_NAME"), DatabaseURLFile: getenv("LMM_STORE_DATABASE_URL_FILE"), StockKeyFile: getenv("LMM_STORE_STOCK_KEY_FILE")}, Support: DatabaseConfig{DatabaseName: getenv("LMM_SUPPORT_DATABASE_NAME"), DatabaseURLFile: getenv("LMM_SUPPORT_DATABASE_URL_FILE")}, Legal: LegalConfig{ConfigFile: getenv("LMM_LEGAL_CONFIG_FILE")}}, nil
}
func absolutePath(path string) bool {
	return path != "" && filepath.IsAbs(path) && filepath.Clean(path) == path && !strings.ContainsAny(path, "\x00\r\n")
}

func LoadConfig(path string) (GoConfig, error) {
	var cfg GoConfig
	if !absolutePath(path) {
		return cfg, errors.New("extension configuration path must be absolute")
	}
	info, err := os.Lstat(path)
	if err != nil || info == nil || !info.Mode().IsRegular() || info.Mode().Perm()&0022 != 0 || info.Size() > 64<<10 {
		return cfg, errors.New("cannot read bounded regular extension TOML configuration")
	}
	f, err := os.Open(path)
	if err != nil {
		return cfg, errors.New("cannot read extension TOML configuration")
	}
	raw, readErr := io.ReadAll(io.LimitReader(f, (64<<10)+1))
	closeErr := f.Close()
	if readErr != nil || closeErr != nil || len(raw) > 64<<10 || !utf8.Valid(raw) {
		return cfg, errors.New("invalid extension TOML configuration file")
	}
	meta, err := toml.NewDecoder(bytes.NewReader(raw)).Decode(&cfg)
	if err != nil || len(meta.Undecoded()) != 0 {
		return cfg, errors.New("invalid or unknown extension TOML configuration field")
	}
	// The decoder's struct matching also accepts case variants. Only canonical
	// names belong to this versioned operator contract.
	allowed := map[string]bool{"version": true, "host": true, "host.listen": true, "host.local_http": true, "host.public_origin": true, "host.token_file": true, "host.plugin_state_file": true, "host.modules": true, "host.web_compat": true, "host.max_in_flight": true, "core": true, "core.socket": true, "core.identity_token_file": true, "core.commerce_token_file": true, "store": true, "store.database_name": true, "store.database_url_file": true, "store.stock_key_file": true, "store.extore_key_file": true, "store.extore_redirect_uri": true, "support": true, "support.database_name": true, "support.database_url_file": true, "legal": true, "legal.config_file": true}
	for _, key := range meta.Keys() {
		if paymentConfigurationKey(key.String()) || toolMarketConfigurationKey(key.String()) || supportCustomersConfigurationKey(key.String()) {
			continue
		}
		switch key.String() {
		case "hero_sms", "hero_sms.core_socket", "hero_sms.core_token_file", "market_review", "market_review.core_socket", "market_review.core_token_file", "market_review.settings_file",
			"core.user_read_socket", "core.user_read_token_file", "core.assistant_token_file", "assistant", "assistant.database_name", "assistant.database_url_file", "assistant.cursor_key_file", "assistant.agent_required",
			"operations", "operations.database_name", "operations.database_url_file", "ionet", "ionet.key_file",
			"community", "community.database_name", "community.database_url_file",
			"acquisition", "acquisition.database_name", "acquisition.database_url_file",
			"scripts", "scripts.root", "scripts.repository_key_file", "scripts.manifest", "scripts.manifest.name", "scripts.manifest.sha256":
			continue
		}
		if !allowed[key.String()] {
			return cfg, errors.New("unknown extension TOML configuration field")
		}
	}
	if cfg.Version != 1 || !absolutePath(cfg.Host.TokenFile) {
		return cfg, errors.New("extension TOML requires version 1 and an absolute host token_file")
	}
	if !meta.IsDefined("host", "max_in_flight") {
		cfg.Host.MaxInFlight = modules.DefaultMaxInFlight
	}
	if cfg.Host.MaxInFlight < 1 || cfg.Host.MaxInFlight > modules.MaximumMaxInFlight {
		return cfg, errors.New("extension max_in_flight must be between 1 and 64")
	}
	if cfg.Host.Listen == "" {
		cfg.Host.Listen = "0.0.0.0:8081"
	}
	if _, _, err = net.SplitHostPort(cfg.Host.Listen); err != nil {
		return cfg, errors.New("invalid extension host listen address")
	}
	for _, p := range []string{cfg.Host.PluginStateFile, cfg.Core.Socket, cfg.Core.IdentityTokenFile, cfg.Core.CommerceTokenFile, cfg.Core.DirectoryAdTokenFile, cfg.Core.AssistantTokenFile, cfg.Core.UserReadTokenFile, cfg.Core.UserReadSocket, cfg.Store.DatabaseURLFile, cfg.Store.StockKeyFile, cfg.Store.ExtoreKeyFile, cfg.Support.DatabaseURLFile, cfg.Legal.ConfigFile, cfg.Assistant.DatabaseURLFile, cfg.Assistant.CursorKeyFile, cfg.Operations.DatabaseURLFile, cfg.Community.DatabaseURLFile, cfg.Acquisition.DatabaseURLFile, cfg.Scripts.Root, cfg.Scripts.RepositoryKeyFile, cfg.IoNet.KeyFile} {
		if p != "" && !absolutePath(p) {
			return cfg, errors.New("extension TOML file and socket paths must be absolute")
		}
	}
	if (cfg.Core.Socket == "") != (cfg.Core.IdentityTokenFile == "") {
		return cfg, errors.New("configure both core socket and identity_token_file")
	}
	if (cfg.Core.UserReadSocket == "") != (cfg.Core.UserReadTokenFile == "") {
		return cfg, errors.New("configure both core user_read_socket and user_read_token_file")
	}
	if cfg.Store.StockKeyFile != "" && (cfg.Store.DatabaseName == "" || cfg.Store.DatabaseURLFile == "") {
		return cfg, errors.New("store stock_key_file requires configured store database_name and database_url_file")
	}
	if (cfg.Store.ExtoreKeyFile == "") != (cfg.Store.ExtoreRedirectURI == "") {
		return cfg, errors.New("configure both store extore_key_file and extore_redirect_uri")
	}
	if cfg.Store.ExtoreKeyFile != "" && (cfg.Store.DatabaseName == "" || cfg.Store.DatabaseURLFile == "") {
		return cfg, errors.New("store Extore import requires configured store database_name and database_url_file")
	}
	if cfg.Store.ExtoreRedirectURI != "" {
		if !strings.HasPrefix(cfg.Store.ExtoreRedirectURI, "https://") || strings.ContainsAny(cfg.Store.ExtoreRedirectURI, " \t\r\n") {
			return cfg, errors.New("store extore_redirect_uri must be an https callback")
		}
	}
	if cfg.Store.StockKeyFile != "" && cfg.Store.ExtoreKeyFile != "" && cfg.Store.StockKeyFile == cfg.Store.ExtoreKeyFile {
		return cfg, errors.New("store seal keys must be independent private paths")
	}
	for _, secret := range []struct {
		path string
		name string
	}{{cfg.Store.StockKeyFile, "store stock_key_file"}, {cfg.Store.ExtoreKeyFile, "store extore_key_file"}} {
		if secret.path == "" {
			continue
		}
		for _, other := range []string{cfg.Host.TokenFile, cfg.Core.IdentityTokenFile, cfg.Core.CommerceTokenFile, cfg.Core.PaymentTokenFile, cfg.Core.AssistantTokenFile, cfg.Core.DirectoryAdTokenFile, cfg.Core.UserReadTokenFile, cfg.Store.DatabaseURLFile, cfg.Assistant.CursorKeyFile, cfg.ToolMarket.EncryptionKeyFile, cfg.ToolMarket.CoreTokenFile} {
			if other != "" && other == secret.path {
				return cfg, errors.New(secret.name + " must be an independent private path")
			}
		}
	}
	for _, db := range []DatabaseConfig{{DatabaseName: cfg.Store.DatabaseName, DatabaseURLFile: cfg.Store.DatabaseURLFile}, cfg.Support, cfg.Operations, cfg.Community, cfg.Acquisition} {
		if (db.DatabaseName == "") != (db.DatabaseURLFile == "") {
			return cfg, errors.New("configure both extension database_name and database_url_file")
		}
	}
	if cfg.Scripts.Root != "" || cfg.Scripts.Manifest != nil {
		if cfg.Scripts.Root == "" || cfg.Scripts.Manifest == nil {
			return cfg, errors.New("configure both scripts root and an explicit reviewed manifest")
		}
	}
	if err := validateSupportCustomersConfiguration(cfg); err != nil {
		return cfg, err
	}
	if err := validateHostProfile(cfg); err != nil {
		return cfg, err
	}
	if err := validateToolMarketConfiguration(cfg); err != nil {
		return cfg, err
	}
	if err := validateHeroSMSConfiguration(cfg); err != nil {
		return cfg, err
	}
	if err := validateMarketReviewConfiguration(cfg); err != nil {
		return cfg, err
	}
	if err := validateAssistantConfiguration(cfg); err != nil {
		return cfg, err
	}
	if err := validatePaymentsConfiguration(cfg); err != nil {
		return cfg, err
	}
	return cfg, nil
}
