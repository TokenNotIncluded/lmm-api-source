package assistant_services

import "testing"

func TestAvailableBindingsDoNotAdvertiseMissingDependencies(t *testing.T) {
	service := &Service{}
	available := service.AvailableBindings()
	expected := map[string]bool{"recall_memory": true, "remember_memory": true, "remember_profile_skill": true, "forget_profile_skill": true, "get_site_issues": true, "show_chart": true, "show_statistics": true, "show_choices": true, "show_flowchart": true, "calculate_math": true, "calculate_cost": true, "get_ui_preference_options": true, "set_ui_preferences": true, "restore_ui_preferences": true}
	if len(available) != len(expected) {
		t.Fatalf("available count=%d", len(available))
	}
	for _, binding := range available {
		if !expected[binding.Name] {
			t.Fatalf("unavailable tool advertised: %s", binding.Name)
		}
		delete(expected, binding.Name)
	}
	if len(expected) != 0 {
		t.Fatalf("missing local tools: %v", expected)
	}
	if service.ConfirmationAvailable() {
		t.Fatal("missing Core proof port advertised")
	}
	if len(service.Bindings()) != 29 {
		t.Fatalf("full frozen manifest=%d", len(service.Bindings()))
	}
}

func TestAvailableBindingsRegisterConfiguredSearchWithoutOtherAuthority(t *testing.T) {
	provider, err := NewSearchProvider(SearchConfig{Provider: "exa", Endpoint: "http://127.0.0.1:1", APIKey: "synthetic-exa-key", LocalHTTP: true})
	if err != nil {
		t.Fatal(err)
	}
	before := (&Service{}).AvailableBindings()
	after := (&Service{dependencies: Dependencies{Search: provider}}).AvailableBindings()
	if len(after) != len(before)+1 {
		t.Fatal("search injection changed unrelated tool availability")
	}
	previous := map[string]bool{}
	for _, binding := range before {
		previous[binding.Name] = true
	}
	search := 0
	for _, binding := range after {
		if binding.Name == "search_web" {
			search++
		} else if !previous[binding.Name] {
			t.Fatal("search injection granted another remote tool")
		}
	}
	if search != 1 {
		t.Fatal("configured search was not registered exactly once")
	}
}
