package modules

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
