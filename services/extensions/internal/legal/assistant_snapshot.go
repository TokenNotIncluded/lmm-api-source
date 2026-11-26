package legal

import "errors"

// PublishedSnapshot is the same immutable, operator-published text served by
// ServeHTTP. It never reads a draft, fetches a URL or permits a policy mutation.
type PublishedSnapshot struct {
	Document            string
	RequestedLanguage   string
	SourceLanguage      string
	Text                string
	Revision            string
	RequestedConfigured bool
	SourceConfigured    bool
}

func (h *Handler) Snapshot(name, language string) (PublishedSnapshot, error) {
	if h == nil || !knownDocument(name) {
		return PublishedSnapshot{}, errors.New("invalid published policy document")
	}
	language = normalizeLanguage(language)
	if language != "" && !languagePattern.MatchString(language) {
		return PublishedSnapshot{}, errors.New("invalid policy language")
	}
	selected, document := h.selectDocument(name, language)
	_, configured := h.documents[name][language]
	return PublishedSnapshot{Document: name, RequestedLanguage: language, SourceLanguage: selected, Text: document.text, Revision: document.revision, RequestedConfigured: configured, SourceConfigured: document.text != ""}, nil
}
