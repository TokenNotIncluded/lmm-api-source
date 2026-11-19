package extore

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"time"
	"unsafe"
)

const maxCatalogBytes = 16 << 20
const maxNativeCertificateBytes = 256 << 10

// CatalogReader replaces only the authorized catalog transport. OAuth and the
// Store's connection ownership, grant, issuer and product checks remain in Go.
type CatalogReader interface {
	ReadCatalog(context.Context, string, string) ([]byte, error)
}

// NativeConfig explicitly selects the installed Extore zplugin and its Rust
// host bridge. Empty configuration retains the existing HTTPS catalog client.
type NativeConfig struct {
	HostLibrary   string `toml:"host_library"`
	PluginLibrary string `toml:"plugin_library"`
	CAFile        string `toml:"ca_file"`
	HostSHA256    string `toml:"host_sha256"`
	PluginSHA256  string `toml:"plugin_sha256"`
}

func (c NativeConfig) Enabled() bool {
	return c.HostLibrary != "" || c.PluginLibrary != "" || c.CAFile != "" || c.HostSHA256 != "" || c.PluginSHA256 != ""
}

func (c NativeConfig) Validate() error {
	if !c.Enabled() {
		return nil
	}
	if c.HostLibrary == "" || c.PluginLibrary == "" {
		return errors.New("store extore_plugin requires both host_library and plugin_library")
	}
	for _, digest := range []string{c.HostSHA256, c.PluginSHA256} {
		raw, err := hex.DecodeString(digest)
		if err != nil || len(raw) != sha256.Size || digest != strings.ToLower(digest) {
			return errors.New("store extore_plugin requires canonical host_sha256 and plugin_sha256")
		}
	}
	for _, path := range []string{c.HostLibrary, c.PluginLibrary, c.CAFile} {
		if path != "" && (!filepath.IsAbs(path) || filepath.Clean(path) != path || strings.TrimSpace(path) != path || strings.ContainsRune(path, 0)) {
			return errors.New("store extore_plugin paths must be absolute")
		}
	}
	return nil
}

// This is extore_plugin_host.h ABI v1, not zplugin's ABI. The Rust bridge alone
// loads zplugin and owns the typed proxy. All pointers are call-scoped except
// nativeBytes.Data, which belongs to nativeBytes.Owner until release.
type nativeBytes struct {
	Data  *byte
	Len   uintptr
	Owner uintptr
}

type nativeAPI struct {
	version func() uint32
	open    func(*byte, uintptr, *byte, uintptr, *byte, uintptr, *uintptr) int32
	catalog func(uintptr, *byte, uintptr, *nativeBytes) int32
	release func(uintptr)
	close   func(uintptr) int32
}

type nativeRequest struct {
	ctx    context.Context
	origin string
	token  string
	result chan nativeResult
}

type nativeResult struct {
	data []byte
	err  error
}

// NativeCatalogReader serializes native work on one dedicated OS thread. A
// mutex cannot provide the thread affinity required by zplugin's !Send proxy.
type NativeCatalogReader struct {
	requests chan nativeRequest
	stop     chan struct{}
	done     chan struct{}
	once     sync.Once
}

func NewNativeCatalogReader(cfg NativeConfig) (*NativeCatalogReader, error) {
	if err := cfg.Validate(); err != nil {
		return nil, err
	}
	if !cfg.Enabled() {
		return nil, errors.New("Extore native catalog is not configured")
	}
	var certificate []byte
	if cfg.CAFile != "" {
		f, err := os.Open(cfg.CAFile)
		if err != nil {
			return nil, errors.New("Extore native CA file could not be opened")
		}
		info, statErr := f.Stat()
		if statErr != nil || !info.Mode().IsRegular() {
			f.Close()
			return nil, errors.New("Extore native CA must be a regular file")
		}
		certificate, err = io.ReadAll(io.LimitReader(f, maxNativeCertificateBytes+1))
		closeErr := f.Close()
		if err != nil || closeErr != nil || len(certificate) == 0 || len(certificate) > maxNativeCertificateBytes {
			return nil, errors.New("Extore native CA must contain bounded PEM certificates")
		}
	}
	hostPath, pluginPath, err := pinNativeLibraries(cfg)
	if err != nil {
		return nil, err
	}
	// Load the exact verified open files, not a second resolution of operator
	// paths. The two descriptors share the native images' process lifetime.
	cfg.HostLibrary, cfg.PluginLibrary = hostPath, pluginPath
	r := &NativeCatalogReader{requests: make(chan nativeRequest), stop: make(chan struct{}), done: make(chan struct{})}
	ready := make(chan error, 1)
	go r.run(cfg, certificate, ready)
	if err := <-ready; err != nil {
		<-r.done
		return nil, err
	}
	return r, nil
}

func (r *NativeCatalogReader) run(cfg NativeConfig, certificate []byte, ready chan<- error) {
	runtime.LockOSThread()
	// Exit without UnlockOSThread so this dedicated thread is retired and its
	// Rust thread-local loader state cannot migrate to another Go reader.
	defer close(r.done)
	api, err := loadNativeAPI(cfg.HostLibrary)
	if err != nil {
		ready <- err
		return
	}
	if api.version() != 1 {
		ready <- errors.New("unsupported Extore native host ABI")
		return
	}
	// Loading and configuring a real typed proxy proves the installed plugin
	// matches the host before Store becomes ready. It performs no HTTP request.
	handle, err := api.openCatalog(cfg.PluginLibrary, DefaultOrigin, certificate)
	if err == nil {
		err = nativeStatus("startup close", api.close(handle))
	}
	ready <- err
	if err != nil {
		return
	}
	for {
		select {
		case <-r.stop:
			return
		case req := <-r.requests:
			select {
			case <-r.stop:
				req.result <- nativeResult{err: ErrUnavailable}
				return
			default:
			}
			if err := req.ctx.Err(); err != nil {
				req.result <- nativeResult{err: err}
				continue
			}
			data, err := api.read(cfg.PluginLibrary, req.origin, certificate, req.token)
			// read releases the result and closes the proxy before handing data
			// back. Cancellation never leaves a native handle with the caller.
			req.result <- nativeResult{data: data, err: err}
		}
	}
}

func (r *NativeCatalogReader) ReadCatalog(ctx context.Context, origin, token string) ([]byte, error) {
	if r == nil {
		return nil, ErrUnavailable
	}
	normalized, err := NormalizeOrigin(origin)
	if err != nil || normalized != origin || token == "" {
		return nil, ErrInput
	}
	// Include worker admission in the existing catalog request budget. A caller
	// can stop waiting while the worker completes same-thread native cleanup.
	ctx, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	req := nativeRequest{ctx: ctx, origin: origin, token: token, result: make(chan nativeResult, 1)}
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	case <-r.stop:
		return nil, ErrUnavailable
	case <-r.done:
		return nil, ErrUnavailable
	case r.requests <- req:
	}
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	case result := <-req.result:
		return result.data, result.err
	case <-r.done:
		return nil, ErrUnavailable
	}
}

// Close waits for the active synchronous native call (bounded by the plugin's
// HTTP timeout) and retires its worker. Native images stay pinned until process
// exit together with their two verified file descriptors; changing the library
// paths or digests requires a process restart. This method does not promise
// dlclose or interrupt an in-flight request.
func (r *NativeCatalogReader) Close() error {
	if r != nil {
		r.once.Do(func() { close(r.stop) })
		<-r.done
	}
	return nil
}

func (a *nativeAPI) openCatalog(path, origin string, certificate []byte) (uintptr, error) {
	pathBytes, originBytes := []byte(path), []byte(origin)
	var handle uintptr
	status := a.open(unsafe.SliceData(pathBytes), uintptr(len(pathBytes)),
		unsafe.SliceData(originBytes), uintptr(len(originBytes)),
		unsafe.SliceData(certificate), uintptr(len(certificate)), &handle)
	runtime.KeepAlive(pathBytes)
	runtime.KeepAlive(originBytes)
	runtime.KeepAlive(certificate)
	if err := nativeStatus("open", status); err != nil {
		return 0, err
	}
	if handle == 0 {
		return 0, fmt.Errorf("Extore native open returned an empty handle: %w", ErrProtocol)
	}
	return handle, nil
}

func (a *nativeAPI) read(path, origin string, certificate []byte, token string) (data []byte, err error) {
	handle, err := a.openCatalog(path, origin, certificate)
	if err != nil {
		return nil, err
	}
	defer func() {
		if closeErr := nativeStatus("close", a.close(handle)); closeErr != nil {
			data, err = nil, errors.Join(err, closeErr)
		}
	}()
	tokenBytes := []byte(token)
	var output nativeBytes
	status := a.catalog(handle, unsafe.SliceData(tokenBytes), uintptr(len(tokenBytes)), &output)
	runtime.KeepAlive(tokenBytes)
	if output.Owner != 0 {
		defer a.release(output.Owner)
	}
	if err := nativeStatus("catalog", status); err != nil {
		return nil, err
	}
	if output.Data == nil || output.Owner == 0 || output.Len == 0 || output.Len > maxCatalogBytes {
		return nil, fmt.Errorf("Extore native catalog returned an invalid buffer: %w", ErrProtocol)
	}
	return append([]byte(nil), unsafe.Slice(output.Data, int(output.Len))...), nil
}

func nativeStatus(operation string, status int32) error {
	if status == 0 {
		return nil
	}
	kind := ErrUnavailable
	switch status {
	case 1001:
		kind = ErrDenied
	case 1003:
		kind = ErrProtocol
	}
	// The fixed operation and numeric ABI status are safe to report. Do not
	// include native response bodies, bearer tokens or authorization payloads.
	return fmt.Errorf("Extore native %s failed (%d): %w", operation, status, kind)
}

// pinNativeLibraries retains one verified library pair for process lifetime.
// dlopen caches names; closing a /proc/self/fd path and reusing that descriptor
// number for a different image could otherwise return the old native mapping.
var nativeLibraryPins struct {
	sync.Mutex
	key   [4]string
	files [2]*os.File
	paths [2]string
}

func pinNativeLibraries(cfg NativeConfig) (string, string, error) {
	key := [4]string{cfg.HostLibrary, cfg.HostSHA256, cfg.PluginLibrary, cfg.PluginSHA256}
	nativeLibraryPins.Lock()
	defer nativeLibraryPins.Unlock()
	if nativeLibraryPins.files[0] != nil {
		if nativeLibraryPins.key != key {
			return "", "", errors.New("Extore native library paths and digests are pinned; restart to change them")
		}
		return nativeLibraryPins.paths[0], nativeLibraryPins.paths[1], nil
	}
	host, err := verifiedNativeLibrary(cfg.HostLibrary, cfg.HostSHA256, "host")
	if err != nil {
		return "", "", err
	}
	plugin, err := verifiedNativeLibrary(cfg.PluginLibrary, cfg.PluginSHA256, "plugin")
	if err != nil {
		host.Close()
		return "", "", err
	}
	nativeLibraryPins.key = key
	nativeLibraryPins.files = [2]*os.File{host, plugin}
	nativeLibraryPins.paths = [2]string{
		fmt.Sprintf("/proc/self/fd/%d", host.Fd()),
		fmt.Sprintf("/proc/self/fd/%d", plugin.Fd()),
	}
	return nativeLibraryPins.paths[0], nativeLibraryPins.paths[1], nil
}

func verifiedNativeLibrary(path, digest, role string) (*os.File, error) {
	file, err := os.Open(path)
	if err != nil {
		return nil, fmt.Errorf("Extore native %s library could not be opened", role)
	}
	valid := false
	defer func() {
		if !valid {
			file.Close()
		}
	}()
	info, err := file.Stat()
	const maxLibraryBytes = 512 << 20
	if err != nil || !info.Mode().IsRegular() || info.Mode().Perm()&0022 != 0 || info.Size() == 0 || info.Size() > maxLibraryBytes {
		return nil, fmt.Errorf("Extore native %s must be a bounded regular library without group or world write access", role)
	}
	hash := sha256.New()
	count, err := io.Copy(hash, io.LimitReader(file, maxLibraryBytes+1))
	if err != nil || count != info.Size() || hex.EncodeToString(hash.Sum(nil)) != digest {
		return nil, fmt.Errorf("Extore native %s library does not match its configured SHA-256", role)
	}
	valid = true
	return file, nil
}
