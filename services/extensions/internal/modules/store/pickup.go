// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package store

import (
	"context"
	"crypto/pbkdf2"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/hex"
	"errors"
	"strings"
	"unicode/utf8"
)

type pickupProtection struct {
	Salt string `json:"salt"`
	Hash string `json:"hash"`
}

type pickupLink struct {
	Token string `json:"token"`
}

func protectPickup(code string) (*pickupProtection, error) {
	if code == "" {
		return nil, nil
	}
	if len(code) < 8 || len(code) > 72 || !utf8.ValidString(code) || strings.ContainsRune(code, 0) {
		return nil, ErrInvalid
	}
	var salt [16]byte
	if _, err := rand.Read(salt[:]); err != nil {
		return nil, err
	}
	hash, err := pbkdf2.Key(sha256.New, code, salt[:], 600000, 32)
	if err != nil {
		return nil, err
	}
	return &pickupProtection{Salt: hex.EncodeToString(salt[:]), Hash: hex.EncodeToString(hash)}, nil
}

func validPickupCode(protection *pickupProtection, code string) bool {
	if protection == nil {
		return code == ""
	}
	if len(code) < 8 || len(code) > 72 {
		return false
	}
	salt, saltErr := hex.DecodeString(protection.Salt)
	wanted, hashErr := hex.DecodeString(protection.Hash)
	if saltErr != nil || hashErr != nil || len(salt) != 16 || len(wanted) != 32 {
		return false
	}
	actual, err := pbkdf2.Key(sha256.New, code, salt, 600000, 32)
	return err == nil && subtle.ConstantTimeCompare(wanted, actual) == 1
}

func readOrderPickup(tx Tx, order Order) (*pickupProtection, error) {
	if order.Legacy == nil || !order.Legacy.PickupCodeRequired {
		return nil, nil
	}
	var protection pickupProtection
	if err := tx.Get("commands", ID("legacy-order-pickup-v1", order.ID), &protection); err != nil {
		return nil, err
	}
	return &protection, nil
}

func checkOrderPickup(tx Tx, order Order, code string) error {
	protection, err := readOrderPickup(tx, order)
	if err != nil {
		return err
	}
	if !validPickupCode(protection, code) {
		return ErrConflict
	}
	return nil
}

func (s *Service) legacyPickupLink(ctx context.Context, token string, actor Account, id string) (any, error) {
	row, err := s.LocateOrder(ctx, id)
	if err != nil {
		return nil, err
	}
	order, err := s.GetOrder(ctx, token, row.ShopID, row.Order.ID)
	if err != nil {
		return nil, err
	}
	if order.Buyer != actor || order.Legacy == nil {
		return nil, ErrForbidden
	}
	var link pickupLink
	err = s.repo.Within(ctx, row.ShopID, func(tx Tx) error {
		key := ID("store-pickup-v1", id)
		if err := tx.Get("commands", key, &link); err == nil {
			return nil
		} else if !errors.Is(err, ErrNotFound) {
			return err
		}
		var secret [32]byte
		if _, err := rand.Read(secret[:]); err != nil {
			return err
		}
		link.Token = id + "." + hex.EncodeToString(secret[:])
		return tx.Put("commands", key, link)
	})
	return map[string]any{"pickup_url": "/store/claim/" + link.Token}, err
}

func (s *Service) Pickup(ctx context.Context, session string, user int64, token, code string, collect bool) (map[string]any, error) {
	id, secret, found := strings.Cut(token, ".")
	if !found || len(id) != 64 || len(secret) != 64 {
		return nil, ErrNotFound
	}
	if _, err := hex.DecodeString(id + secret); err != nil {
		return nil, ErrNotFound
	}
	if collect && user == 0 {
		return nil, ErrUnauthorized
	}
	if user > 0 {
		if _, err := s.check(ctx, session, Account{Kind: "personal", ID: user}, Read); err != nil {
			return nil, err
		}
	}
	row, err := s.LocateOrder(ctx, id)
	if err != nil {
		return nil, err
	}
	var order Order
	var protection *pickupProtection
	err = s.repo.Within(ctx, row.ShopID, func(tx Tx) error {
		var link pickupLink
		if err := tx.Get("commands", ID("store-pickup-v1", id), &link); err != nil {
			return err
		}
		if subtle.ConstantTimeCompare([]byte(link.Token), []byte(token)) != 1 {
			return ErrNotFound
		}
		if err := tx.Get("orders", id, &order); err != nil {
			return err
		}
		var err error
		protection, err = readOrderPickup(tx, order)
		return err
	})
	if err != nil {
		return nil, err
	}
	if order.Legacy == nil || user > 0 && order.Buyer != (Account{Kind: "personal", ID: user}) {
		return nil, ErrNotFound
	}
	view, err := legacyOrderView(LocatedOrder{Order: order})
	if err != nil {
		return nil, err
	}
	if !collect {
		return map[string]any{"order_id": order.ID, "product_title": order.ProductTitle, "variant_id": order.VariantID, "variant_name": order.Title, "delivery_template": order.Legacy.DeliveryTemplate, "quantity": order.Quantity, "status": view["status"], "pickup_login_required": true, "pickup_login_satisfied": user > 0, "pickup_code_required": order.Legacy.PickupCodeRequired}, nil
	}
	if !validPickupCode(protection, code) {
		return nil, ErrForbidden
	}
	if order.State != Paid && order.State != Fulfilled && order.State != SettlementPending && order.State != Settled || order.RefundState == "requested" {
		return nil, ErrConflict
	}
	if !order.Delivered {
		order, err = s.FinalizeDelivery(ctx, row.ShopID, order.ID)
		if err != nil {
			return nil, err
		}
	}
	// Refunds can advance while password verification or Core completion is in
	// flight. Read the final delivery under the same shop lock as refund changes.
	var result map[string]any
	err = s.repo.Within(ctx, row.ShopID, func(tx Tx) error {
		if err := tx.Get("orders", id, &order); err != nil {
			return err
		}
		if order.State != Paid && order.State != Fulfilled && order.State != SettlementPending && order.State != Settled || order.RefundState == "requested" {
			return ErrConflict
		}
		if !order.Delivered || !Text(order.Delivery, 16000) {
			return ErrPending
		}
		result = map[string]any{"order_id": order.ID, "trade_no": order.ID, "product_id": order.Legacy.ProductID, "product_title": order.ProductTitle, "variant_id": order.VariantID, "variant_name": order.Title, "delivery_template": order.Legacy.DeliveryTemplate, "quantity": order.Quantity, "fixed_content": order.Delivery, "items": []string{}}
		return nil
	})
	return result, err
}
