package payments

import (
	"sync"
	"sync/atomic"
)

type pancakeConfigurationSnapshot struct {
	wallet    map[int64]Adapter
	adapters  map[int64]*Pancake
	admission map[int64]bool
}

// PancakeConfiguration is attached before either service begins serving.
// Published maps are immutable and shared by wallet and subscription readers.
type PancakeConfiguration struct {
	mu      sync.Mutex
	current atomic.Pointer[pancakeConfigurationSnapshot]
}

func AttachPancakeConfiguration(wallet *Service, subscriptions *PancakeSubscriptions) *PancakeConfiguration {
	c := &PancakeConfiguration{}
	snapshot := &pancakeConfigurationSnapshot{wallet: wallet.adapters}
	if subscriptions != nil {
		snapshot.adapters, snapshot.admission = subscriptions.adapters, subscriptions.admission
		subscriptions.configuration = c
	}
	c.current.Store(snapshot)
	wallet.configuration = c
	return c
}

func (s *Service) adapterSnapshot() map[int64]Adapter {
	if s.configuration != nil {
		return s.configuration.current.Load().wallet
	}
	return s.adapters
}

func (s *PancakeSubscriptions) pancakeSnapshot() *pancakeConfigurationSnapshot {
	if s.configuration != nil {
		return s.configuration.current.Load()
	}
	return &pancakeConfigurationSnapshot{adapters: s.adapters, admission: s.admission}
}

// Update serializes validation, persistence and publication. prepare must build
// all adapters without mutating live state. Publication cannot fail after disk
// persistence succeeds.
func (c *PancakeConfiguration) Update(prepare func() ([]Adapter, map[int64]bool, error), persist func() error) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	adapters, admission, err := prepare()
	if err != nil {
		return err
	}
	old := c.current.Load()
	next := &pancakeConfigurationSnapshot{wallet: make(map[int64]Adapter), adapters: make(map[int64]*Pancake), admission: make(map[int64]bool)}
	for id, a := range old.wallet {
		next.wallet[id] = a
	}
	for id, a := range old.adapters {
		next.adapters[id] = a
	}
	for id, allowed := range old.admission {
		next.admission[id] = allowed
	}
	for _, a := range adapters {
		p, ok := a.(*Pancake)
		if !ok {
			return ErrInvalid
		}
		prior, ok := old.wallet[p.Channel().ID].(*Pancake)
		if !ok || prior.Channel() != p.Channel() {
			return ErrConflict
		}
		next.wallet[p.Channel().ID] = p
		if old.adapters[p.Channel().ID] != nil {
			next.adapters[p.Channel().ID] = p
		}
		next.admission[p.Channel().ID] = admission[p.Channel().ID]
	}
	if err = persist(); err != nil {
		return err
	}
	c.current.Store(next)
	return nil
}
