// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"errors"
	"fmt"
	"html"
	"math"
	"net/url"
	"regexp"
	"strconv"
	"strings"
)

var hexColor = regexp.MustCompile(`^#[0-9a-fA-F]{6}$`)

type svgOptions struct {
	Width, Height, Radius                         int
	Period, Format, Font, Lang                    string
	Background, Foreground, Accent, Muted, Border string
	Title, Footer                                 string
	ShowRequests, CustomTitle, CustomFooter       bool
}

var svgPeriods = map[string]map[string]string{
	"en":    {"7d": "Last 7 days", "30d": "Last 30 days", "365d": "Last 365 days", "all": "All time"},
	"zh":    {"7d": "近 7 天", "30d": "近 30 天", "365d": "近 365 天", "all": "全部时间"},
	"zh-TW": {"7d": "近 7 天", "30d": "近 30 天", "365d": "近 365 天", "all": "全部時間"},
	"fr":    {"7d": "7 derniers jours", "30d": "30 derniers jours", "365d": "365 derniers jours", "all": "Depuis le début"},
	"ja":    {"7d": "過去 7 日", "30d": "過去 30 日", "365d": "過去 365 日", "all": "全期間"},
	"ru":    {"7d": "За 7 дней", "30d": "За 30 дней", "365d": "За 365 дней", "all": "За всё время"},
	"vi":    {"7d": "7 ngày qua", "30d": "30 ngày qua", "365d": "365 ngày qua", "all": "Từ trước đến nay"},
}

// parseAggregateSVGOptions ports the legacy aggregate (badge-based) options.
func parseAggregateSVGOptions(q url.Values) (svgOptions, error) {
	for key := range q {
		switch key {
		case "layout", "width", "height", "radius", "period", "format", "theme", "font", "animation", "bg", "fg", "accent", "muted", "border", "title", "label", "footer", "requests", "lang":
		default:
			return svgOptions{}, fmt.Errorf("unknown option %s", key)
		}
	}
	get := func(key, fallback string) string {
		if v := q.Get(key); v != "" {
			return v
		}
		return fallback
	}
	o := svgOptions{Period: get("period", "30d"), Format: get("format", "compact"), Font: get("font", "sans"), Lang: get("lang", "en"), ShowRequests: true}
	theme := get("theme", "dark")
	for _, f := range []struct{ value, allowed string }{{o.Period, "7d,30d,365d,all"}, {o.Format, "compact,full"}, {theme, "paper,dark,transparent"}, {o.Font, "sans,mono,serif"}, {get("animation", "wave"), "wave,pulse,none"}} {
		if !strings.Contains(","+f.allowed+",", ","+f.value+",") {
			return svgOptions{}, errors.New("invalid option value")
		}
	}
	if _, ok := svgPeriods[o.Lang]; !ok {
		return svgOptions{}, errors.New("invalid lang")
	}
	o.Footer = svgPeriods[o.Lang][o.Period]
	if theme == "dark" {
		o.Background, o.Foreground, o.Accent, o.Muted, o.Border = "#202020", "#f3f3f3", "#dedede", "#aaa9a8", "#363636"
	} else {
		o.Background, o.Foreground, o.Accent, o.Muted, o.Border = "#faf9f5", "#17231f", "#d97757", "#6c746d", "#d9ded5"
	}
	if theme == "transparent" {
		o.Background = "none"
	}
	integer := func(key string, fallback, min, max int) (int, error) {
		v := q.Get(key)
		if v == "" {
			return fallback, nil
		}
		n, err := strconv.Atoi(v)
		if err != nil || n < min || n > max {
			return 0, fmt.Errorf("invalid %s", key)
		}
		return n, nil
	}
	var err error
	if o.Width, err = integer("width", 1200, 480, 1600); err != nil {
		return o, err
	}
	// Height 0 derives the canvas from the bounded source count.
	if o.Height, err = integer("height", 0, 200, 1200); err != nil {
		return o, err
	}
	if o.Radius, err = integer("radius", 20, 0, 48); err != nil {
		return o, err
	}
	for _, c := range []struct {
		key    string
		target *string
	}{{"bg", &o.Background}, {"fg", &o.Foreground}, {"accent", &o.Accent}, {"muted", &o.Muted}, {"border", &o.Border}} {
		if v := q.Get(c.key); v != "" {
			if !hexColor.MatchString(v) {
				return o, fmt.Errorf("invalid %s color", c.key)
			}
			*c.target = v
		}
	}
	if v := q.Get("requests"); v != "" {
		if v != "0" && v != "1" {
			return o, errors.New("invalid requests")
		}
		o.ShowRequests = v == "1"
	}
	if v := q.Get("title"); v != "" {
		if o.Title, err = cleanText(v, 60); err != nil {
			return o, errors.New("invalid title")
		}
		o.CustomTitle = true
	}
	if v := q.Get("footer"); v != "" {
		if o.Footer, err = cleanText(v, 48); err != nil {
			return o, errors.New("invalid footer")
		}
		o.CustomFooter = true
	}
	if v := q.Get("label"); v != "" {
		if _, err = cleanText(v, 32); err != nil {
			return o, errors.New("invalid label")
		}
	}
	return o, nil
}

func svgText(value string) string {
	return html.EscapeString(strings.Map(func(r rune) rune {
		if r == '\t' || r == '\n' || r == '\r' || (r >= 0x20 && r <= 0xd7ff) || (r >= 0xe000 && r <= 0xfffd) || (r >= 0x10000 && r <= 0x10ffff) {
			return r
		}
		return -1
	}, value))
}

func svgShort(value string, maxWidth int) string {
	width := 0
	var b strings.Builder
	for _, r := range value {
		step := 1
		if r > 0x2e7f {
			step = 2
		}
		if width+step > maxWidth {
			b.WriteRune('…')
			break
		}
		b.WriteRune(r)
		width += step
	}
	return b.String()
}

func formatNumber(value int64, format, lang string) string {
	if format == "full" {
		return strconv.FormatInt(value, 10)
	}
	type unit struct {
		threshold float64
		suffix    string
	}
	units := []unit{{1e12, "T"}, {1e9, "B"}, {1e6, "M"}, {1e3, "K"}}
	switch lang {
	case "zh":
		units = []unit{{1e8, "亿"}, {1e4, "万"}}
	case "zh-TW", "ja":
		units = []unit{{1e8, "億"}, {1e4, "万"}}
	}
	for _, u := range units {
		if float64(value) >= u.threshold {
			return strings.TrimSuffix(strconv.FormatFloat(float64(value)/u.threshold, 'f', 1, 64), ".0") + u.suffix
		}
	}
	return strconv.FormatInt(value, 10)
}

type aggregateCopy struct{ Title, Tokens, Requests, Messages, Snapshot, Live, Unknown, Login, Unsupported, Disabled, Observed, Fetched, All, Windows string }

var aggregateLanguages = map[string]aggregateCopy{
	"en":    {"My AI usage", "tokens", "requests", "messages", "Owner snapshot", "Live public data", "Unavailable", "Sign-in required", "Snapshot required", "Disabled", "Observed", "Fetched", "Lifetime", "Each source keeps its own period; totals are not combined."},
	"zh":    {"我的 AI 用量", "Token", "请求", "消息", "用户快照", "公开数据", "不可用", "需登录查看", "需要导入快照", "未启用", "观察于", "读取于", "累计", "各来源保留各自统计期间，不合并成一个总数。"},
	"zh-TW": {"我的 AI 用量", "Token", "請求", "訊息", "使用者快照", "公開資料", "無法取得", "需登入查看", "需要匯入快照", "未啟用", "觀察於", "讀取於", "累計", "各來源保留各自統計期間，不合併成一個總數。"},
	"fr":    {"Mon utilisation IA", "tokens", "requêtes", "messages", "Instantané du propriétaire", "Données publiques", "Indisponible", "Connexion requise", "Instantané requis", "Désactivé", "Observé", "Consulté", "Cumul", "Chaque source conserve sa période ; les totaux ne sont pas additionnés."},
	"ja":    {"AI 利用状況", "トークン", "リクエスト", "メッセージ", "所有者のスナップショット", "公開データ", "取得不可", "ログインが必要", "スナップショットが必要", "未有効", "確認日時", "取得日時", "累計", "各サービスの集計期間を保持し、合計しません。"},
	"ru":    {"Использование ИИ", "токенов", "запросов", "сообщений", "Снимок владельца", "Открытые данные", "Недоступно", "Нужен вход", "Нужен снимок", "Отключено", "Наблюдение", "Получено", "За всё время", "У каждого источника свой период; итоги не складываются."},
	"vi":    {"Mức dùng AI của tôi", "token", "yêu cầu", "tin nhắn", "Ảnh chụp của chủ hồ sơ", "Dữ liệu công khai", "Không có dữ liệu", "Cần đăng nhập", "Cần nhập ảnh chụp", "Đã tắt", "Quan sát", "Đã lấy", "Tích lũy", "Mỗi nguồn giữ kỳ riêng; không cộng các tổng."},
}

// renderAggregateSVG is the legacy aggregate card renderer.
func renderAggregateSVG(o svgOptions, sources []aggregateSource) string {
	c := aggregateLanguages[o.Lang]
	if len(sources) > profileShareMaxLinkedProfiles+1 {
		sources = sources[:profileShareMaxLinkedProfiles+1]
	}
	contentHeight := 190 + len(sources)*140
	height := o.Height
	if height == 0 {
		height = min(max(int(math.Ceil(float64(contentHeight)*float64(o.Width)/1200)), 200), 1200)
	}
	scale := math.Min(float64(o.Width)/1200, float64(height)/float64(contentHeight))
	x, y := (float64(o.Width)-1200*scale)/2, (float64(height)-float64(contentHeight)*scale)/2
	title := c.Title
	if o.CustomTitle {
		title = o.Title
	}
	font := "Arial, Noto Sans CJK SC, sans-serif"
	if o.Font == "mono" {
		font = "ui-monospace, Noto Sans Mono CJK SC, monospace"
	} else if o.Font == "serif" {
		font = "Georgia, Noto Serif CJK SC, serif"
	}
	var svg strings.Builder
	fmt.Fprintf(&svg, `<svg xmlns="http://www.w3.org/2000/svg" width="%d" height="%d" viewBox="0 0 %d %d" role="img" aria-labelledby="aggregate-title aggregate-desc"><title id="aggregate-title">%s</title><desc id="aggregate-desc">%s</desc><rect x="1" y="1" width="%d" height="%d" rx="%d" fill="%s" stroke="%s"/><g transform="translate(%.4f %.4f) scale(%.6f)" font-family="%s">`, o.Width, height, o.Width, height, svgText(title), svgText(c.Windows), o.Width-2, height-2, o.Radius, o.Background, o.Border, x, y, scale, font)
	text := func(x, y, size int, anchor, color, value string) {
		fmt.Fprintf(&svg, `<text x="%d" y="%d" font-size="%d" text-anchor="%s" fill="%s">%s</text>`, x, y, size, anchor, color, svgText(value))
	}
	text(48, 65, 30, "start", o.Foreground, svgShort(title, 58))
	text(1152, 63, 18, "end", o.Accent, "LMM Best")
	text(48, 99, 15, "start", o.Muted, svgShort(c.Windows, 120))
	for i, row := range sources {
		y := 125 + i*140
		fmt.Fprintf(&svg, `<path d="M48 %dH1152" stroke="%s"/><g><title>%s · %s · %s</title>`, y, o.Border, svgText(row.Label), svgText(row.URL), svgText(row.SnapshotSource))
		text(48, y+36, 22, "start", o.Foreground, svgShort(row.Label, 38))
		status := map[string]string{"live": c.Live, "snapshot": c.Snapshot, "login_required": c.Login, "unsupported": c.Unsupported, "disabled": c.Disabled}[row.Status]
		if status == "" {
			status = c.Unknown
		}
		text(48, y+66, 15, "start", o.Muted, status)
		period := row.Period
		if row.Period == "all" {
			period = c.All
		} else if row.PeriodStart != "" && row.PeriodEnd != "" {
			period = row.PeriodStart + " — " + row.PeriodEnd
			if row.PeriodTimezone == "UTC" {
				period += " UTC"
			}
		} else if row.Status == "snapshot" && len(row.ObservedAt) >= 10 && row.Period != "unknown" {
			period = row.Period + " @ " + row.ObservedAt[:10]
		} else if p, ok := svgPeriods[o.Lang][row.Period]; ok {
			period = p
		}
		text(48, y+96, 13, "start", o.Muted, svgShort(period, 68))
		metric := func(value *int64, label string) string {
			if value == nil {
				return "— " + label
			}
			prefix := ""
			if row.Approximate {
				prefix = "≈"
			}
			return prefix + formatNumber(*value, o.Format, o.Lang) + " " + label
		}
		main := metric(row.Tokens, c.Tokens)
		if row.Tokens == nil && row.Messages != nil {
			main = metric(row.Messages, c.Messages)
		} else if row.Tokens == nil && row.Requests != nil && o.ShowRequests {
			main = metric(row.Requests, c.Requests)
		}
		text(1152, y+39, int(math.Min(31, 600/math.Max(1, float64(len([]rune(main)))*.62))), "end", o.Accent, main)
		var extra []string
		if o.ShowRequests && row.Requests != nil && (row.Tokens != nil || row.Messages != nil) {
			extra = append(extra, metric(row.Requests, c.Requests))
		}
		if row.Messages != nil && row.Tokens != nil {
			extra = append(extra, metric(row.Messages, c.Messages))
		}
		if len(extra) > 0 {
			text(1152, y+66, 15, "end", o.Foreground, svgShort(strings.Join(extra, " · "), 68))
		}
		if row.ObservedAt != "" {
			text(1152, y+96, 13, "end", o.Muted, c.Observed+" "+row.ObservedAt)
		} else {
			text(1152, y+96, 13, "end", o.Muted, "")
		}
		if row.SnapshotSource != "" {
			text(48, y+123, 12, "start", o.Muted, svgShort(row.SnapshotSource, 150))
		}
		svg.WriteString(`</g>`)
	}
	footer := "api.lmm.best · profile-share"
	if o.CustomFooter {
		footer = o.Footer
	}
	text(48, contentHeight-25, 13, "start", o.Muted, svgShort(footer, 80))
	svg.WriteString(`</g></svg>`)
	return svg.String()
}
