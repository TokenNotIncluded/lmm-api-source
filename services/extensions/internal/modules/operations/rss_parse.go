// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
// Adapted from the fixed go-v0.2.118 RSS reader in TokenNotIncluded/lmm-api.
package operations

import (
	"encoding/xml"
	"html"
	"net/url"
	"regexp"
	"sort"
	"strings"
	"time"
)

// Explicit XML field names also cover RDF's namespaced item elements.
type rssEntryXML struct {
	Title       string `xml:"title"`
	Link        string `xml:"link"`
	GUID        string `xml:"guid"`
	Description string `xml:"description"`
	Content     string `xml:"encoded"`
	PubDate     string `xml:"pubDate"`
	Date        string `xml:"date"`
}

type atomEntryXML struct {
	ID        string `xml:"id"`
	Title     string `xml:"title"`
	Summary   string `xml:"summary"`
	Content   string `xml:"content"`
	Published string `xml:"published"`
	Updated   string `xml:"updated"`
	Links     []struct {
		Rel  string `xml:"rel,attr"`
		Href string `xml:"href,attr"`
	} `xml:"link"`
}

var rssTags = regexp.MustCompile(`(?s)<[^>]*>`)

func parseRSS(body []byte, feedURL string) ([]RSSItem, error) {
	var document struct {
		XMLName xml.Name
		Channel struct {
			Items []rssEntryXML `xml:"item"`
		} `xml:"channel"`
		Items   []rssEntryXML  `xml:"item"`
		Entries []atomEntryXML `xml:"entry"`
	}
	if xml.Unmarshal(body, &document) != nil {
		return nil, ErrInvalid
	}
	items := []RSSItem{}
	add := func(id, title, link, summary, date string) {
		link = rssItemURL(feedURL, link)
		title = rssText(title)
		if title == "" {
			title = link
		}
		if title == "" {
			return
		}
		published := rssDate(date)
		id = rssClip(firstRSS(id, link, title+"|"+published), 512)
		items = append(items, RSSItem{ID: id, Title: title, URL: link, Summary: rssText(summary), PublishedAt: published})
	}
	switch strings.ToLower(document.XMLName.Local) {
	case "rss", "rdf":
		entries := document.Channel.Items
		if strings.EqualFold(document.XMLName.Local, "rdf") {
			entries = document.Items
		}
		for _, entry := range entries {
			add(entry.GUID, entry.Title, entry.Link, firstRSS(entry.Description, entry.Content), firstRSS(entry.PubDate, entry.Date))
		}
	case "feed":
		for _, entry := range document.Entries {
			var link string
			for _, candidate := range entry.Links {
				if candidate.Rel == "" || candidate.Rel == "alternate" {
					link = candidate.Href
					break
				}
			}
			add(entry.ID, entry.Title, link, firstRSS(entry.Summary, entry.Content), firstRSS(entry.Published, entry.Updated))
		}
	default:
		return nil, ErrInvalid
	}
	sort.SliceStable(items, func(i, j int) bool { return items[i].PublishedAt > items[j].PublishedAt })
	if len(items) > 50 {
		items = items[:50]
	}
	return items, nil
}

func rssItemURL(feedURL, raw string) string {
	base, err := url.Parse(feedURL)
	if err != nil || strings.TrimSpace(raw) == "" {
		return ""
	}
	link, err := base.Parse(strings.TrimSpace(raw))
	if err != nil || (link.Scheme != "http" && link.Scheme != "https") || link.Hostname() == "" || link.User != nil || len(link.String()) > 2048 {
		return ""
	}
	return link.String()
}

func rssDate(raw string) string {
	for _, layout := range []string{time.RFC3339Nano, time.RFC1123Z, time.RFC1123, time.RFC822Z, time.RFC822, "Mon, 2 Jan 2006 15:04:05 -0700", "2006-01-02T15:04:05Z0700"} {
		if value, err := time.Parse(layout, strings.TrimSpace(raw)); err == nil {
			return value.UTC().Format(time.RFC3339)
		}
	}
	return ""
}

func rssText(raw string) string {
	return rssClip(strings.Join(strings.Fields(html.UnescapeString(rssTags.ReplaceAllString(raw, " "))), " "), 600)
}

func rssClip(raw string, limit int) string {
	runes := []rune(raw)
	if len(runes) > limit {
		return string(runes[:limit]) + "…"
	}
	return raw
}

func firstRSS(values ...string) string {
	for _, value := range values {
		if value = strings.TrimSpace(value); value != "" {
			return value
		}
	}
	return ""
}
