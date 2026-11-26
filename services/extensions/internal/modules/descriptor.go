package modules

import (
	"fmt"
	"slices"
	"strings"
)

// Descriptor declares a compiled plugin's identity, dependencies and Web entry
// points. Composition uses the same declaration for selection and inventory;
// it grants neither Core authority nor permission to execute unreviewed code.
type Descriptor struct {
	Name            string
	Version         string
	Requires        []string
	NavigationPaths []string
}

func (d Descriptor) info() PluginInfo {
	return PluginInfo{
		Name: d.Name, Version: d.Version, APIVersion: PluginAPIVersion,
		RegistrationSHA256: RegistrationSHA256(d.Name, d.Version),
		Requires:           append([]string(nil), d.Requires...),
		NavigationPaths:    append([]string(nil), d.NavigationPaths...),
	}
}

// ValidatePluginCatalog checks the full compiled catalog, including modules
// not selected on this host. Every valid graph can be enabled and disabled one
// module at a time without implicitly changing another module's state.
func ValidatePluginCatalog(catalog []Descriptor) error {
	byName := make(map[string]Descriptor, len(catalog))
	for _, d := range catalog {
		if !validName.MatchString(d.Name) || d.Name == "modules" || !pluginVersionPattern.MatchString(d.Version) {
			return fmt.Errorf("%w: invalid name or version for %q", ErrPluginManifest, d.Name)
		}
		if _, duplicate := byName[d.Name]; duplicate {
			return fmt.Errorf("%w: duplicate registration %q", ErrPluginManifest, d.Name)
		}
		byName[d.Name] = d
	}

	const (
		visiting = iota + 1
		done
	)
	state := make(map[string]int, len(catalog))
	var visit func(string, []string) error
	visit = func(name string, path []string) error {
		switch state[name] {
		case done:
			return nil
		case visiting:
			cycle := append(path[slices.Index(path, name):], name)
			return fmt.Errorf("%w: dependency cycle: %s", ErrPluginManifest, strings.Join(cycle, " -> "))
		}
		state[name] = visiting
		path = append(path, name)
		required := byName[name].Requires
		seen := make(map[string]bool, len(required))
		for _, dep := range required {
			if _, exists := byName[dep]; !exists {
				return fmt.Errorf("%w: %s requires unknown plugin %q", ErrPluginManifest, name, dep)
			}
			if seen[dep] {
				return fmt.Errorf("%w: duplicate dependency %q in %s", ErrPluginManifest, dep, name)
			}
			seen[dep] = true
			if err := visit(dep, path); err != nil {
				return err
			}
		}
		state[name] = done
		return nil
	}
	for _, d := range catalog {
		if err := visit(d.Name, nil); err != nil {
			return err
		}
	}
	return nil
}
