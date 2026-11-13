package store

import (
	"bytes"
	"context"
	"crypto/tls"
	"crypto/x509"
	"database/sql"
	"encoding/json"
	"errors"
	_ "github.com/lib/pq"
	"net"
	"net/mail"
	"net/smtp"
	"net/textproto"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"
)

func mailPG(t *testing.T) (*sql.DB, *Postgres, *PickupSMTP, PickupSMTPConfig, []byte) {
	t.Helper()
	path := os.Getenv("STORE_MAIL_FIXTURE")
	if path == "" {
		t.Skip("private real PG/TLS SMTP fixture required")
	}
	raw, err := os.ReadFile(path)
	must(t, err)
	var f struct {
		DSNFile string           `json:"dsn_file"`
		SMTP    PickupSMTPConfig `json:"smtp"`
	}
	must(t, json.Unmarshal(raw, &f))
	raw, err = os.ReadFile(f.DSNFile)
	must(t, err)
	db, err := sql.Open("postgres", strings.TrimSpace(string(raw)))
	must(t, err)
	t.Cleanup(func() { db.Close() })
	db.SetMaxOpenConns(4)
	var privileged bool
	must(t, db.QueryRow(`SELECT rolsuper OR rolcreatedb OR rolcreaterole OR rolbypassrls FROM pg_roles WHERE rolname=current_user`).Scan(&privileged))
	if privileged {
		t.Fatal("runtime must not be privileged")
	}
	repo, err := NewPostgres(db)
	must(t, err)
	ca, err := os.ReadFile(f.SMTP.CAFile)
	must(t, err)
	sender, err := NewPickupSMTP(f.SMTP, "", ca)
	must(t, err)
	key, err := os.ReadFile(f.SMTP.KeyFile)
	must(t, err)
	return db, repo, sender, f.SMTP, key
}
func TestPickupMailRestartProcess(t *testing.T) {
	id := os.Getenv("STORE_MAIL_CHILD_ORDER")
	if id == "" {
		t.Skip("owned restart helper")
	}
	_, repo, sender, cfg, key := mailPG(t)
	s, err := New(repo, &authority{}, nil)
	must(t, err)
	must(t, s.WithPickupMail(sender, key, cfg.Origin))
	state, err := s.DispatchPickupMail(context.Background(), os.Getenv("STORE_MAIL_CHILD_SHOP"), id)
	must(t, err)
	t.Log("persisted mail state", state)
}
func TestPickupMailRealPostgresTLS(t *testing.T) {
	db, repo, sender, cfg, key := mailPG(t)
	ctx := context.Background()
	f, policy := limitFixture(t, 100, 10)
	promo := testPromotion(f, 10000, 100)
	writePromotion(t, f, promo)
	// Copy synthetic catalogue/terms into real PG; every purchase/outbox thereafter
	// uses real PG transactions. Free orders invoke no synthetic financial port.
	must(t, f.repo.Within(ctx, f.shop.ID, func(mem Tx) error {
		return repo.Within(ctx, f.shop.ID, func(pg Tx) error {
			for _, kind := range []string{"shops", "products", "variants", "commands"} {
				rows, err := mem.List(kind, "", 100)
				if err != nil {
					return err
				}
				for _, raw := range rows {
					var body map[string]any
					if json.Unmarshal(raw, &body) != nil {
						t.Fatal("fixture decode")
					}
					id, _ := body["id"].(string)
					if kind == "commands" {
						continue
					}
					if kind == "products" {
						var p Product
						must(t, json.Unmarshal(raw, &p))
						p.Legacy.EmailPickupLink = true
						must(t, pg.Put(kind, p.ID, p))
					} else {
						must(t, pg.Put(kind, id, raw))
					}
				}
			}
			must(t, pg.Put("commands", "@web:seller-terms", legacySellerTerms{Version: policy.Version, Content: "Synthetic terms", UpdatedAt: 1}))
			return pg.Put("commands", "@web:promotions:"+f.product.ID, promotionList{Items: []Promotion{promo}})
		})
	}))
	service := func() *Service {
		s, err := New(repo, f.auth, nil)
		must(t, err)
		must(t, s.WithPickupMail(sender, key, cfg.Origin))
		return s
	}
	s := service()
	purchase := func(label, email string) Order {
		t.Helper()
		o, err := s.Purchase(ctx, buyerToken, Purchase{ShopID: f.shop.ID, VariantID: f.variant.ID, Buyer: Account{"personal", 2}, Quantity: 1, Key: label, Policy: policy, PromotionCode: promo.Code, RequireFree: true, PickupEmail: email})
		must(t, err)
		return o
	}
	record := func(id string) pickupMailRecord {
		t.Helper()
		var r pickupMailRecord
		must(t, repo.Within(ctx, f.shop.ID, func(tx Tx) error { return tx.Get("commands", pickupMailID(id), &r) }))
		return r
	}
	dispatch := func(s *Service, id, want string) {
		t.Helper()
		state, err := s.DispatchPickupMail(ctx, f.shop.ID, id)
		must(t, err)
		if state != want {
			t.Fatalf("state=%s want=%s", state, want)
		}
	}
	accept := purchase("mail-accept", "accept@example.test")
	var wg sync.WaitGroup
	for i := 0; i < 8; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			state, err := service().DispatchPickupMail(ctx, f.shop.ID, accept.ID)
			if err != nil || (state != "accepted_by_smtp" && state != "dispatching") {
				t.Errorf("claim state=%s err=%v", state, err)
			}
		}()
	}
	wg.Wait()
	if record(accept.ID).State != "accepted_by_smtp" {
		t.Fatal("acceptance not persisted")
	}
	dispatch(service(), accept.ID, "accepted_by_smtp")
	reject := purchase("mail-reject", "reject@example.test")
	dispatch(s, reject.ID, "rejected")
	dispatch(service(), reject.ID, "rejected")
	retry := purchase("mail-retry", "retry@example.test")
	dispatch(s, retry.ID, "retry_wait")
	attempt := record(retry.ID).Attempt
	dispatch(service(), retry.ID, "retry_wait")
	if record(retry.ID).Attempt != attempt {
		t.Fatal("ignored backoff")
	}
	// Advance only the owned fixture's backoff clock, then recover durable retry
	// with a new service. The receiver refuses its first RCPT and accepts the next.
	must(t, repo.Within(ctx, f.shop.ID, func(tx Tx) error {
		r := recordUnsafe(t, tx, retry.ID)
		r.NextAttemptAt = time.Now().Add(-time.Second).Unix()
		return tx.Put("commands", pickupMailID(retry.ID), r)
	}))
	dispatch(service(), retry.ID, "accepted_by_smtp")
	if record(retry.ID).Attempt == attempt {
		t.Fatal("due retry did not obtain a fresh attempt")
	}
	// An explicit DATA 450 permits retry, but the frozen wire message must not change.
	dated := purchase("mail-date-retry", "dataretry@example.test")
	dispatch(s, dated.ID, "retry_wait")
	must(t, repo.Within(ctx, f.shop.ID, func(tx Tx) error {
		r := recordUnsafe(t, tx, dated.ID)
		r.NextAttemptAt = time.Now().Add(-time.Second).Unix()
		return tx.Put("commands", pickupMailID(dated.ID), r)
	}))
	dispatch(service(), dated.ID, "accepted_by_smtp")
	firstWire, err := os.ReadFile(filepath.Join(os.Getenv("STORE_MAIL_SPOOL_DIR"), "dataretry-first.eml"))
	must(t, err)
	paths, err := filepath.Glob(filepath.Join(os.Getenv("STORE_MAIL_SPOOL_DIR"), "spool-*"))
	must(t, err)
	matched := false
	for _, path := range paths {
		raw, err := os.ReadFile(path)
		must(t, err)
		wire := bytes.SplitN(raw, []byte("\n--accepted-copy--\n"), 2)[0]
		msg, err := mail.ReadMessage(bytes.NewReader(wire))
		must(t, err)
		date, err := msg.Header.Date()
		must(t, err)
		if msg.Header.Get("Message-ID") == "<"+pickupMailID(dated.ID)+"@lmm-store.invalid>" {
			matched = true
			if !date.Equal(time.Unix(dated.Legacy.CreatedAt, 0)) || !bytes.Equal(firstWire, wire) {
				t.Fatal("DATA retry changed frozen RFC Date/Message-ID/body/token")
			}
		}
	}
	if !matched {
		t.Fatal("dated retry absent from real TLS sink")
	}
	t.Log("actual TLS sink net/mail.Header.Date parsed; DATA retry preserved exact wire bytes and original order date")
	lost := purchase("mail-lost", "lost@example.test")
	dispatch(s, lost.ID, "unknown")
	dispatch(service(), lost.ID, "unknown")
	pending := purchase("mail-process-restart", "accept@example.test")
	child := func(id string) *exec.Cmd {
		cmd := exec.Command(os.Args[0], "-test.run=^TestPickupMailRestartProcess$", "-test.v", "-test.count=1")
		cmd.Env = append(os.Environ(), "STORE_MAIL_CHILD_ORDER="+id, "STORE_MAIL_CHILD_SHOP="+f.shop.ID)
		return cmd
	}
	raw, err := child(pending.ID).CombinedOutput()
	if err != nil {
		t.Fatalf("restart subprocess: %v %s", err, raw)
	}
	if record(pending.ID).State != "accepted_by_smtp" {
		t.Fatal("fresh process did not recover pending")
	}
	crash := purchase("mail-crash", "hang@example.test")
	cmd := child(crash.ID)
	must(t, cmd.Start())
	end := time.Now().Add(5 * time.Second)
	for record(crash.ID).State != "dispatching" && time.Now().Before(end) {
		time.Sleep(10 * time.Millisecond)
	}
	if record(crash.ID).State != "dispatching" {
		_ = cmd.Process.Kill()
		_ = cmd.Wait()
		t.Fatal("child never claimed")
	}
	must(t, cmd.Process.Kill())
	_ = cmd.Wait()
	must(t, repo.Within(ctx, f.shop.ID, func(tx Tx) error {
		r := recordUnsafe(t, tx, crash.ID)
		r.StartedAt = time.Now().Add(-31 * time.Second).Unix()
		return tx.Put("commands", pickupMailID(crash.ID), r)
	}))
	dispatch(service(), crash.ID, "unknown")
	dispatch(service(), crash.ID, "unknown")
	canceled := purchase("mail-cancel-network", "hang@example.test")
	short, stop := context.WithTimeout(ctx, 200*time.Millisecond)
	defer stop()
	start := time.Now()
	state, err := s.DispatchPickupMail(short, f.shop.ID, canceled.ID)
	must(t, err)
	if time.Since(start) > 2*time.Second || state != "unknown" {
		t.Fatalf("cancellation failed state=%s", state)
	}
	// Persisted ciphertext decrypts under the same key after restart and is a real claim link.
	must(t, repo.Within(ctx, f.shop.ID, func(tx Tx) error {
		_, p, err := service().openPickupMail(tx, f.shop.ID, accept.ID)
		if err != nil {
			return err
		}
		if !strings.HasPrefix(p.URL, cfg.Origin+"/store/claim/") {
			t.Fatal("bad link")
		}
		return nil
	}))
	var delivered pickupMailPayload
	must(t, repo.Within(ctx, f.shop.ID, func(tx Tx) error {
		_, p, err := service().openPickupMail(tx, f.shop.ID, accept.ID)
		delivered = p
		return err
	}))
	claim, err := service().Pickup(ctx, "", 0, delivered.Token, "", true)
	must(t, err)
	if claim["fixed_content"] != "Synthetic delivery" {
		t.Fatal("real PG mail token did not claim the purchased goods")
	}
	// Roll back both the encrypted outbox and digest in a real PG transaction.
	rolled := ID("mail-rollback")
	err = repo.Within(ctx, f.shop.ID, func(tx Tx) error {
		if e := s.preparePickupMail(tx, f.shop.ID, Order{ID: rolled}, "accept@example.test"); e != nil {
			return e
		}
		return ErrConflict
	})
	if !errors.Is(err, ErrConflict) {
		t.Fatal("rollback witness failed")
	}
	must(t, repo.Within(ctx, f.shop.ID, func(tx Tx) error {
		var r pickupMailRecord
		if e := tx.Get("commands", pickupMailID(rolled), &r); !errors.Is(e, ErrNotFound) {
			t.Fatal("outbox survived rollback")
		}
		var link pickupLink
		if e := tx.Get("commands", ID("store-pickup-v1", rolled), &link); !errors.Is(e, ErrNotFound) {
			t.Fatal("digest survived rollback")
		}
		return nil
	}))
	wrong := service()
	must(t, wrong.WithPickupMail(sender, []byte(strings.Repeat("z", 32)), cfg.Origin))
	_, err = wrong.DispatchPickupMail(ctx, f.shop.ID, accept.ID)
	if !errors.Is(err, ErrUnavailable) {
		t.Fatal("wrong key silently accepted")
	}
	purchase("mail-runtime-worker", "accept@example.test")
	var count int
	must(t, db.QueryRow(`SELECT count(*) FROM lmm_store.commands WHERE body->>'kind'='store-pickup-mail-v1'`).Scan(&count))
	if count != 9 {
		t.Fatalf("outbox rows %d want9", count)
	}
	t.Log("real PG + TLS SMTP: accepted/rejected/retry/lost DATA ACK, multi-instance ownership, process restart, crash recovery and cancellation; no money port")
}
func recordUnsafe(t *testing.T, tx Tx, id string) pickupMailRecord {
	t.Helper()
	var r pickupMailRecord
	must(t, tx.Get("commands", pickupMailID(id), &r))
	return r
}

func TestPickupSMTPRealTLSModes(t *testing.T) {
	_, _, sender, cfg, _ := mailPG(t)
	raw, err := os.ReadFile(os.Getenv("STORE_MAIL_FIXTURE"))
	must(t, err)
	var fixture struct {
		StartTLSPort int `json:"starttls_port"`
	}
	must(t, json.Unmarshal(raw, &fixture))
	cfg.Port = fixture.StartTLSPort
	cfg.TLSMode = "starttls"
	ca, err := os.ReadFile(cfg.CAFile)
	must(t, err)
	starttls, err := NewPickupSMTP(cfg, "", ca)
	must(t, err)
	payload := pickupMailPayload{CreatedAt: 1700000000, Recipient: "accept@example.test", URL: "https://store.example.test/store/claim/synthetic_tls_only"}
	if got := starttls.send(context.Background(), ID("starttls-accept"), payload); got.state != "accepted_by_smtp" {
		t.Fatalf("STARTTLS result %+v", got)
	}
	untrusted := sender.config
	untrusted.CAFile = ""
	bad, err := NewPickupSMTP(untrusted, "", nil)
	must(t, err)
	if got := bad.send(context.Background(), ID("untrusted-cert"), payload); got.state != "retry_wait" {
		t.Fatalf("untrusted certificate accepted %+v", got)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	started := time.Now()
	if got := sender.send(ctx, ID("already-canceled"), payload); got.state != "retry_wait" || time.Since(started) > time.Second {
		t.Fatal("pre-network cancellation failed")
	}
}

// deferPlainResponse exercises the standard library's PLAIN response through a
// real 334 challenge instead of its optional SASL initial response. PLAIN's
// credential construction and TLS/name checks remain in smtp.PlainAuth.
type deferPlainResponse struct {
	base     smtp.Auth
	response []byte
}

func (a *deferPlainResponse) Start(info *smtp.ServerInfo) (string, []byte, error) {
	mechanism, response, err := a.base.Start(info)
	a.response = response
	return mechanism, nil, err
}
func (a *deferPlainResponse) Next(challenge []byte, more bool) ([]byte, error) {
	if more && len(challenge) == 0 && a.response != nil {
		response := a.response
		a.response = nil
		return response, nil
	}
	return a.base.Next(challenge, more)
}
func TestPickupSMTPRealAuthPlain(t *testing.T) {
	path := os.Getenv("STORE_MAIL_AUTH_FIXTURE")
	if path == "" {
		t.Skip("private real TLS SMTP AUTH fixture required")
	}
	raw, err := os.ReadFile(path)
	must(t, err)
	var f struct {
		SMTP         PickupSMTPConfig `json:"smtp"`
		ImplicitPort int              `json:"implicit_port"`
		StartTLSPort int              `json:"starttls_port"`
		PlainPort    int              `json:"plain_port"`
		Events       string           `json:"events"`
	}
	must(t, json.Unmarshal(raw, &f))
	ca, err := os.ReadFile(f.SMTP.CAFile)
	must(t, err)
	password, err := os.ReadFile(f.SMTP.PasswordFile)
	must(t, err)
	payload := pickupMailPayload{CreatedAt: 1700000000, Recipient: "accept@example.test", URL: "https://store.example.test/store/claim/synthetic_auth_only"}
	for _, mode := range []string{"implicit", "starttls"} {
		t.Run("sender_"+mode, func(t *testing.T) {
			cfg := f.SMTP
			cfg.TLSMode = mode
			cfg.Port = f.ImplicitPort
			if mode == "starttls" {
				cfg.Port = f.StartTLSPort
			}
			sender, err := NewPickupSMTP(cfg, string(password), ca)
			must(t, err)
			if got := sender.send(context.Background(), ID("auth-success", mode), payload); got.state != "accepted_by_smtp" {
				t.Fatalf("authenticated sender result %+v", got)
			}
			sender, err = NewPickupSMTP(cfg, "synthetic-incorrect-password", ca)
			must(t, err)
			if got := sender.send(context.Background(), ID("auth-bad-password", mode), payload); got.state != "rejected" || got.code != 535 {
				t.Fatalf("incorrect password result %+v", got)
			}
		})
	}
	connect := func(port int, secure bool, name string) *smtp.Client {
		t.Helper()
		conn, err := net.DialTimeout("tcp", net.JoinHostPort("localhost", strconv.Itoa(port)), 3*time.Second)
		must(t, err)
		must(t, conn.SetDeadline(time.Now().Add(3*time.Second)))
		var wire net.Conn = conn
		if secure {
			roots := x509.NewCertPool()
			if !roots.AppendCertsFromPEM(ca) {
				t.Fatal("CA fixture")
			}
			tlsConn := tls.Client(conn, &tls.Config{MinVersion: tls.VersionTLS12, ServerName: "localhost", RootCAs: roots})
			must(t, tlsConn.Handshake())
			wire = tlsConn
		}
		client, err := smtp.NewClient(wire, name)
		must(t, err)
		t.Cleanup(func() { client.Close() })
		return client
	}
	for _, correct := range []bool{true, false} {
		client := connect(f.ImplicitPort, true, "localhost")
		secret := string(password)
		if !correct {
			secret = "synthetic-incorrect-password"
		}
		err := client.Auth(&deferPlainResponse{base: smtp.PlainAuth("", f.SMTP.Username, secret, "localhost")})
		if correct {
			must(t, err)
		} else {
			var reply *textproto.Error
			if !errors.As(err, &reply) || reply.Code != 535 {
				t.Fatal("challenge did not reject incorrect password with 535")
			}
		}
		client.Close()
	}
	// The STARTTLS listener must reject AUTH before any challenge/credential.
	// Deferring the initial response ensures this probe sends no secret material.
	client := connect(f.StartTLSPort, false, "localhost")
	err = client.Auth(&deferPlainResponse{base: smtp.PlainAuth("", f.SMTP.Username, string(password), "localhost")})
	var reply *textproto.Error
	if !errors.As(err, &reply) || reply.Code != 530 {
		t.Fatal("server admitted AUTH before TLS")
	}
	client.Close()
	// Even a listener advertising AUTH without STARTTLS must receive no AUTH
	// from the production sender: StartTLS fails before client.Auth is reached.
	cfg := f.SMTP
	cfg.TLSMode = "starttls"
	cfg.Port = f.PlainPort
	sender, err := NewPickupSMTP(cfg, string(password), ca)
	must(t, err)
	if got := sender.send(context.Background(), ID("auth-no-tls"), payload); got.state != "retry_wait" {
		t.Fatalf("no-STARTTLS result %+v", got)
	}
	// Standard PLAIN also refuses credential disclosure on an unencrypted remote
	// identity, independent of the server's willingness to accept AUTH.
	client = connect(f.PlainPort, false, "smtp.example.test")
	if client.Auth(smtp.PlainAuth("", f.SMTP.Username, string(password), "smtp.example.test")) == nil {
		t.Fatal("PLAIN disclosed credentials without TLS")
	}
	client.Close()
	raw, err = os.ReadFile(f.Events)
	must(t, err)
	var inline, challenge, success, rejected int
	for _, line := range bytes.Split(bytes.TrimSpace(raw), []byte("\n")) {
		var event struct {
			Event     string `json:"event"`
			TLS       bool   `json:"tls"`
			Challenge bool   `json:"challenge"`
			Success   bool   `json:"success"`
		}
		must(t, json.Unmarshal(line, &event))
		if !event.TLS {
			t.Fatal("sink observed AUTH/credentials before TLS")
		}
		switch event.Event {
		case "plain":
			if event.Challenge {
				challenge++
			} else {
				inline++
			}
		case "result":
			if event.Success {
				success++
			} else {
				rejected++
			}
		}
	}
	if inline != 4 || challenge != 2 || success != 3 || rejected != 3 {
		t.Fatalf("AUTH evidence inline=%d challenge=%d success=%d rejected=%d", inline, challenge, success, rejected)
	}
	t.Log("real TLS AUTH PLAIN: production sender implicit/STARTTLS correct+wrong password; explicit 334 challenges correct+wrong; pre-TLS AUTH 530; sender no-STARTTLS no AUTH; no plaintext credential disclosure")
}
