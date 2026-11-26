package modules

// This file implements the extension plugin lifecycle. Plugins are trusted,
// compiled Go modules registered by the host binary. The lifecycle is dynamic
// at the route boundary, while code installation remains a reviewed release
// operation; arbitrary shared libraries are never loaded into the process.

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"sort"
	"strings"
	"sync"
)

var (
	ErrPluginNotFound       = errors.New("plugin is not registered")
	ErrPluginNotInstalled   = errors.New("plugin is not installed")
	ErrPluginUnavailable    = errors.New("plugin is not assembled in this host")
	ErrPluginDependency     = errors.New("plugin dependency is not enabled")
	ErrPluginAlreadyEnabled = errors.New("plugin is already enabled")
	ErrPluginAlreadyOff     = errors.New("plugin is already disabled")
	ErrPluginManifest       = errors.New("invalid plugin manifest")
	ErrPluginOrder          = errors.New("invalid plugin load order")
	ErrPluginState          = errors.New("invalid plugin state")
)

const (
	PluginManifestVersion = 1
	PluginAPIVersion      = 1
	maxPluginStateBytes   = 64 << 10
)

var pluginVersionPattern = regexp.MustCompile(`^[0-9A-Za-z][0-9A-Za-z._-]{0,63}$`)

// PluginInfo is the public, non-secret lifecycle view returned to an
// administrator. RegistrationSHA256 binds the request to a compiled plugin
// registration, not to untrusted source supplied over HTTP.
type PluginInfo struct {
	Name               string   `json:"name"`
	Version            string   `json:"version"`
	APIVersion         int      `json:"api_version"`
	RegistrationSHA256 string   `json:"registration_sha256"`
	Available          bool     `json:"available"`
	Installed          bool     `json:"installed"`
	Enabled            bool     `json:"enabled"`
	LoadOrder          int      `json:"load_order,omitempty"`
	RequiredBy         []string `json:"required_by,omitempty"`
	Requires           []string `json:"requires,omitempty"`
	NavigationPaths    []string `json:"navigation_paths,omitempty"`
}

type PluginInstallRequest struct {
	Name               string `json:"name"`
	Version            string `json:"version"`
	APIVersion         int    `json:"api_version"`
	RegistrationSHA256 string `json:"registration_sha256"`
}

type pluginStateFile struct {
	FormatVersion int           `json:"format_version"`
	Plugins       []pluginState `json:"plugins"`
}

type pluginState struct {
	Name               string `json:"name"`
	Version            string `json:"version"`
	APIVersion         int    `json:"api_version"`
	RegistrationSHA256 string `json:"registration_sha256"`
	Installed          bool   `json:"installed"`
	Enabled            bool   `json:"enabled"`
}

type pluginEntry struct {
	info   PluginInfo
	module Module
}

// ModuleReloader is deliberately narrow so lifecycle tests can use a fake and
// the manager cannot mutate storage or bypass Host admission.
type ModuleReloader interface {
	ReloadModules([]string) error
}

// PluginManager owns the enabled snapshot and persists only lifecycle state.
// It never accepts executable bytes, imports a Go plugin, or grants a module
// core authority; those properties are fixed by the compiled registration.
type PluginManager struct {
	mu        sync.RWMutex
	reloader  ModuleReloader
	stateFile string
	entries   map[string]pluginEntry
	installed map[string]bool
	enabled   map[string]bool
	loadOrder []string
}

func RegistrationSHA256(name, version string) string {
	h := sha256.Sum256([]byte("lmm-extension-registration-v1\x00" + name + "\x00" + version))
	return hex.EncodeToString(h[:])
}

func NewPluginManager(reloader ModuleReloader, registered []Module, stateFile string, catalog ...Descriptor) (*PluginManager, error) {
	if reloader == nil {
		return nil, errors.New("plugin manager requires a module reloader")
	}
	if stateFile != "" {
		if !filepath.IsAbs(stateFile) || filepath.Clean(stateFile) != stateFile || strings.ContainsAny(stateFile, "\x00\r\n") {
			return nil, errors.New("plugin state file must be an absolute clean path")
		}
	}
	if err := ValidatePluginCatalog(catalog); err != nil {
		return nil, err
	}
	m := &PluginManager{reloader: reloader, stateFile: stateFile, entries: make(map[string]pluginEntry, len(registered)), installed: make(map[string]bool, len(registered)), enabled: make(map[string]bool, len(registered))}
	for _, descriptor := range catalog {
		m.entries[descriptor.Name] = pluginEntry{info: descriptor.info()}
	}
	for _, module := range registered {
		if isNil(module) || !validName.MatchString(module.Name()) || module.Name() == "modules" || m.entries[module.Name()].module != nil || isNil(module.Handler()) {
			return nil, ErrPluginManifest
		}
		entry, declared := m.entries[module.Name()]
		if len(catalog) != 0 && !declared {
			return nil, ErrPluginManifest
		}
		version := "1.0.0"
		if declared {
			version = entry.info.Version
		}
		if versioned, ok := module.(interface{ Version() string }); ok && versioned.Version() != "" {
			version = versioned.Version()
		}
		if !pluginVersionPattern.MatchString(version) || (declared && version != entry.info.Version) {
			return nil, ErrPluginManifest
		}
		if !declared {
			entry.info = (Descriptor{Name: module.Name(), Version: version}).info()
		}
		entry.module, entry.info.Available = module, true
		m.entries[module.Name()] = entry
	}
	defaultOrder, err := m.sortedOrder()
	if err != nil {
		return nil, err
	}
	m.loadOrder = defaultOrder
	loaded, exists, err := readPluginState(stateFile)
	if err != nil {
		return nil, err
	}
	if exists {
		// Format v1 stored plugins alphabetically. Reuse the entry order for
		// precedence so a rolled-back release can still read the same file.
		seen := make(map[string]bool, len(loaded))
		m.loadOrder = make([]string, 0, len(m.entries))
		for _, state := range loaded {
			seen[state.Name] = true
			m.loadOrder = append(m.loadOrder, state.Name)
		}
		for _, name := range defaultOrder {
			if !seen[name] {
				m.loadOrder = append(m.loadOrder, name)
			}
		}
	}
	if !exists {
		for name, entry := range m.entries {
			m.installed[name], m.enabled[name] = entry.info.Available, entry.info.Available
		}
		if err := m.validateEnabledLocked(); err != nil {
			return nil, err
		}
		if err := m.persistLocked(); err != nil {
			return nil, err
		}
	} else {
		for name := range m.entries {
			m.installed[name], m.enabled[name] = false, false
		}
		for _, state := range loaded {
			entry, ok := m.entries[state.Name]
			if !ok || state.Version != entry.info.Version || state.APIVersion != entry.info.APIVersion || state.RegistrationSHA256 != entry.info.RegistrationSHA256 || (state.Enabled && !state.Installed) {
				return nil, ErrPluginState
			}
			m.installed[state.Name], m.enabled[state.Name] = state.Installed, state.Enabled
		}
	}
	if err := m.reloadLocked(); err != nil {
		return nil, err
	}
	return m, nil
}

func (m *PluginManager) List() []PluginInfo {
	m.mu.RLock()
	defer m.mu.RUnlock()
	return m.listLocked()
}

func (m *PluginManager) listLocked() []PluginInfo {
	result := make([]PluginInfo, 0, len(m.entries))
	for _, name := range m.enabledOrderLocked() {
		info := m.infoLocked(name)
		info.LoadOrder = len(result) + 1
		result = append(result, info)
	}
	for _, name := range m.loadOrder {
		if !m.enabled[name] {
			result = append(result, m.infoLocked(name))
		}
	}
	return result
}

func (m *PluginManager) IsEnabled(name string) bool {
	m.mu.RLock()
	defer m.mu.RUnlock()
	return m.enabledLocked(name)
}

func (m *PluginManager) enabledLocked(name string) bool {
	entry := m.entries[name]
	if !entry.info.Available || !m.enabled[name] {
		return false
	}
	for _, dependency := range entry.info.Requires {
		if !m.entries[dependency].info.Available || !m.enabled[dependency] {
			return false
		}
	}
	return true
}

func (m *PluginManager) Install(request PluginInstallRequest) (PluginInfo, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	entry, ok := m.entries[request.Name]
	if !ok {
		return PluginInfo{}, ErrPluginNotFound
	}
	if !entry.info.Available {
		return PluginInfo{}, ErrPluginUnavailable
	}
	if request.APIVersion != entry.info.APIVersion || request.Version != entry.info.Version || request.RegistrationSHA256 != entry.info.RegistrationSHA256 {
		return PluginInfo{}, ErrPluginManifest
	}
	wasInstalled := m.installed[request.Name]
	m.installed[request.Name] = true
	if err := m.persistLocked(); err != nil {
		m.installed[request.Name] = wasInstalled
		return PluginInfo{}, err
	}
	return m.infoLocked(request.Name), nil
}

func (m *PluginManager) Enable(name string) (PluginInfo, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	entry, ok := m.entries[name]
	if !ok {
		return PluginInfo{}, ErrPluginNotFound
	}
	if !entry.info.Available {
		return PluginInfo{}, ErrPluginUnavailable
	}
	if !m.installed[name] {
		return PluginInfo{}, ErrPluginNotInstalled
	}
	if m.enabled[name] {
		return m.infoLocked(name), ErrPluginAlreadyEnabled
	}
	m.enabled[name] = true
	if err := m.reloadLocked(); err != nil {
		m.enabled[name] = false
		return PluginInfo{}, err
	}
	if err := m.persistLocked(); err != nil {
		m.enabled[name] = false
		_ = m.reloadLocked()
		return PluginInfo{}, err
	}
	return m.infoLocked(entry.info.Name), nil
}

func (m *PluginManager) Disable(name string) (PluginInfo, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if _, ok := m.entries[name]; !ok {
		return PluginInfo{}, ErrPluginNotFound
	}
	if !m.enabled[name] {
		return m.infoLocked(name), ErrPluginAlreadyOff
	}
	m.enabled[name] = false
	if err := m.reloadLocked(); err != nil {
		m.enabled[name] = true
		return PluginInfo{}, err
	}
	if err := m.persistLocked(); err != nil {
		m.enabled[name] = true
		_ = m.reloadLocked()
		return PluginInfo{}, err
	}
	return m.infoLocked(name), nil
}

func (m *PluginManager) Uninstall(name string) (PluginInfo, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if _, ok := m.entries[name]; !ok {
		return PluginInfo{}, ErrPluginNotFound
	}
	if m.enabled[name] {
		return PluginInfo{}, errors.New("disable plugin before uninstall")
	}
	wasInstalled := m.installed[name]
	m.installed[name] = false
	if err := m.persistLocked(); err != nil {
		m.installed[name] = wasInstalled
		return PluginInfo{}, err
	}
	return m.infoLocked(name), nil
}

func (m *PluginManager) Reload() error {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.reloadLocked()
}

func (m *PluginManager) reloadLocked() error {
	if err := m.validateEnabledLocked(); err != nil {
		return err
	}
	return m.reloader.ReloadModules(m.enabledOrderLocked())
}

func (m *PluginManager) enabledOrderLocked() []string {
	order, _ := m.topologicalOrder(m.loadOrder, m.enabled)
	return order
}

// SetLoadOrder accepts a complete permutation of enabled modules and rejects
// requests that would put a dependent ahead of its prerequisite.
func (m *PluginManager) SetLoadOrder(order []string) ([]PluginInfo, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	current := m.enabledOrderLocked()
	if len(order) != len(current) {
		return nil, ErrPluginOrder
	}
	seen := make(map[string]bool, len(order))
	for _, name := range order {
		if !m.enabled[name] || seen[name] {
			return nil, ErrPluginOrder
		}
		for _, dependency := range m.entries[name].info.Requires {
			if !seen[dependency] {
				return nil, ErrPluginOrder
			}
		}
		seen[name] = true
	}
	next := slices.Clone(m.loadOrder)
	index := 0
	for i, name := range next {
		if m.enabled[name] {
			next[i] = order[index]
			index++
		}
	}
	return m.applyLoadOrderLocked(next)
}

// AutoSortLoadOrder restores deterministic dependency order and uses names to
// break ties between modules that can load independently.
func (m *PluginManager) AutoSortLoadOrder() ([]PluginInfo, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	order, err := m.sortedOrder()
	if err != nil {
		return nil, err
	}
	return m.applyLoadOrderLocked(order)
}

func (m *PluginManager) applyLoadOrderLocked(order []string) ([]PluginInfo, error) {
	previous := m.loadOrder
	m.loadOrder = order
	if err := m.reloadLocked(); err != nil {
		m.loadOrder = previous
		return nil, err
	}
	if err := m.persistLocked(); err != nil {
		m.loadOrder = previous
		return nil, errors.Join(err, m.reloadLocked())
	}
	return m.listLocked(), nil
}

func (m *PluginManager) sortedOrder() ([]string, error) {
	names := make([]string, 0, len(m.entries))
	selected := make(map[string]bool, len(m.entries))
	for name := range m.entries {
		names = append(names, name)
		selected[name] = true
	}
	sort.Strings(names)
	return m.topologicalOrder(names, selected)
}

func (m *PluginManager) topologicalOrder(preference []string, selected map[string]bool) ([]string, error) {
	indegree := make(map[string]int, len(selected))
	dependents := make(map[string][]string, len(selected))
	for name, enabled := range selected {
		if !enabled {
			continue
		}
		indegree[name] = len(m.entries[name].info.Requires)
		seen := make(map[string]bool)
		for _, dependency := range m.entries[name].info.Requires {
			if !selected[dependency] || dependency == name || seen[dependency] {
				return nil, fmt.Errorf("%w: %s requires %s", ErrPluginManifest, name, dependency)
			}
			seen[dependency] = true
			dependents[dependency] = append(dependents[dependency], name)
		}
	}
	order := make([]string, 0, len(indegree))
	for len(indegree) > 0 {
		next := ""
		for _, name := range preference {
			if degree, ok := indegree[name]; ok && degree == 0 {
				next = name
				break
			}
		}
		if next == "" {
			return nil, fmt.Errorf("%w: plugin dependency cycle", ErrPluginManifest)
		}
		order = append(order, next)
		delete(indegree, next)
		for _, dependent := range dependents[next] {
			indegree[dependent]--
		}
	}
	return order, nil
}

func (m *PluginManager) validateEnabledLocked() error {
	for name, on := range m.enabled {
		if !on {
			continue
		}
		entry := m.entries[name]
		if !entry.info.Available {
			return fmt.Errorf("%w: %s", ErrPluginUnavailable, name)
		}
		for _, dependency := range entry.info.Requires {
			if !m.enabled[dependency] || !m.entries[dependency].info.Available {
				return fmt.Errorf("%w: %s requires %s", ErrPluginDependency, name, dependency)
			}
		}
	}
	return nil
}

func (m *PluginManager) infoLocked(name string) PluginInfo {
	info := m.entries[name].info
	info.Installed, info.Enabled = m.installed[name], m.enabledLocked(name)
	for dependent, entry := range m.entries {
		if slices.Contains(entry.info.Requires, name) {
			info.RequiredBy = append(info.RequiredBy, dependent)
		}
	}
	sort.Strings(info.RequiredBy)
	info.Requires = append([]string(nil), info.Requires...)
	info.NavigationPaths = append([]string(nil), info.NavigationPaths...)
	return info
}

func (m *PluginManager) persistLocked() error {
	if m.stateFile == "" {
		return nil
	}
	directory := filepath.Dir(m.stateFile)
	if err := os.MkdirAll(directory, 0700); err != nil {
		return fmt.Errorf("create plugin state directory: %w", err)
	}
	if info, err := os.Stat(directory); err != nil || !info.IsDir() || info.Mode().Perm()&0077 != 0 {
		return ErrPluginState
	}
	state := pluginStateFile{FormatVersion: PluginManifestVersion}
	for _, name := range m.loadOrder {
		entry := m.entries[name]
		state.Plugins = append(state.Plugins, pluginState{Name: name, Version: entry.info.Version,
			APIVersion: entry.info.APIVersion, RegistrationSHA256: entry.info.RegistrationSHA256,
			Installed: m.installed[name], Enabled: m.enabled[name]})
	}
	raw, err := json.Marshal(state)
	if err != nil {
		return err
	}
	tmp, err := os.CreateTemp(directory, ".plugins-*.tmp")
	if err != nil {
		return err
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName)
	if err = tmp.Chmod(0600); err == nil {
		_, err = tmp.Write(append(raw, '\n'))
	}
	if err == nil {
		err = tmp.Sync()
	}
	if closeErr := tmp.Close(); err == nil {
		err = closeErr
	}
	if err != nil {
		return err
	}
	if err := os.Rename(tmpName, m.stateFile); err != nil {
		return err
	}
	return nil
}

func readPluginState(path string) ([]pluginState, bool, error) {
	if path == "" {
		return nil, false, nil
	}
	info, err := os.Lstat(path)
	if errors.Is(err, os.ErrNotExist) {
		return nil, false, nil
	}
	if err != nil || info.Mode()&os.ModeSymlink != 0 || !info.Mode().IsRegular() || info.Mode().Perm()&0077 != 0 || info.Size() > maxPluginStateBytes {
		return nil, false, ErrPluginState
	}
	f, err := os.Open(path)
	if err != nil {
		return nil, false, ErrPluginState
	}
	defer f.Close()
	raw, err := io.ReadAll(io.LimitReader(f, maxPluginStateBytes+1))
	if err != nil || len(raw) > maxPluginStateBytes {
		return nil, false, ErrPluginState
	}
	decoder := json.NewDecoder(strings.NewReader(string(raw)))
	decoder.DisallowUnknownFields()
	var state pluginStateFile
	if err := decoder.Decode(&state); err != nil || state.FormatVersion != PluginManifestVersion {
		return nil, false, ErrPluginState
	}
	var extra any
	if err := decoder.Decode(&extra); !errors.Is(err, io.EOF) {
		return nil, false, ErrPluginState
	}
	seen := map[string]bool{}
	for _, item := range state.Plugins {
		if !validName.MatchString(item.Name) || !pluginVersionPattern.MatchString(item.Version) || item.APIVersion != PluginAPIVersion || !validDigest(item.RegistrationSHA256) || seen[item.Name] {
			return nil, false, ErrPluginState
		}
		seen[item.Name] = true
	}
	return state.Plugins, true, nil
}

func validDigest(value string) bool {
	if len(value) != sha256.Size*2 {
		return false
	}
	_, err := hex.DecodeString(value)
	return err == nil
}
