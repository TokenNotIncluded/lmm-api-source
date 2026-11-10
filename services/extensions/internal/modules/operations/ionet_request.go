// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/url"
	"strconv"
	"strings"
)

func ValidIoNetID(id string) bool { return ioNetID.MatchString(id) }
func ValidIoNetName(name string) bool {
	return len(name) > 0 && len(name) <= 128 && strings.TrimSpace(name) == name && !strings.ContainsAny(name, "\x00\r\n")
}

type IoNetContainerConfig struct {
	ReplicaCount int               `json:"replica_count"`
	Env          map[string]string `json:"env_variables,omitempty"`
	Secrets      map[string]string `json:"secret_env_variables,omitempty"`
	Entrypoint   []string          `json:"entrypoint,omitempty"`
	Port         int               `json:"traffic_port,omitempty"`
	Args         []string          `json:"args,omitempty"`
}
type IoNetRegistry struct {
	Image    string `json:"image_url"`
	Username string `json:"registry_username,omitempty"`
	Secret   string `json:"registry_secret,omitempty"`
}
type IoNetCreate struct {
	Name      string               `json:"resource_private_name"`
	Hours     int                  `json:"duration_hours"`
	GPUs      int                  `json:"gpus_per_container"`
	Hardware  int                  `json:"hardware_id"`
	Locations []int                `json:"location_ids"`
	Container IoNetContainerConfig `json:"container_config"`
	Registry  IoNetRegistry        `json:"registry_config"`
}
type IoNetPriceRequest struct {
	Locations    []int  `json:"location_ids"`
	Hardware     int    `json:"hardware_id"`
	GPUs         int    `json:"gpus_per_container"`
	Hours        int    `json:"duration_hours"`
	Replicas     int    `json:"replica_count"`
	Currency     string `json:"currency"`
	DurationType string `json:"duration_type,omitempty"`
	DurationQty  int    `json:"duration_qty,omitempty"`
	HardwareQty  int    `json:"hardware_qty,omitempty"`
}

func (p IoNetPriceRequest) Endpoint() (string, error) {
	if p.Hardware <= 0 || p.GPUs <= 0 || p.Hours <= 0 || p.Replicas <= 0 || len(p.Locations) == 0 || len(p.Locations) > 100 || p.DurationType != "" && p.DurationType != "hour" || p.DurationQty != 0 && p.DurationQty != p.Hours || p.HardwareQty != 0 && p.HardwareQty != p.GPUs {
		return "", ErrInvalid
	}
	for _, id := range p.Locations {
		if id <= 0 {
			return "", ErrInvalid
		}
	}
	if p.Currency == "" {
		p.Currency = "usdc"
	}
	if p.Currency != "usdc" && p.Currency != "io" {
		return "", ErrInvalid
	}
	locations, _ := json.Marshal(p.Locations)
	v := url.Values{"location_ids": {string(locations)}, "hardware_id": {strconv.Itoa(p.Hardware)}, "gpus_per_container": {strconv.Itoa(p.GPUs)}, "duration_hours": {strconv.Itoa(p.Hours)}, "replica_count": {strconv.Itoa(p.Replicas)}, "duration_type": {"hour"}, "duration_qty": {strconv.Itoa(p.Hours)}, "hardware_qty": {strconv.Itoa(p.GPUs)}, "currency": {p.Currency}}
	return "/price?" + v.Encode(), nil
}

type IoNetMutation struct {
	Resource, Method, Endpoint string
	Body                       json.RawMessage
}

func IoNetMutationFields(kind string) []string {
	switch kind {
	case "create":
		return []string{"resource_private_name", "duration_hours", "gpus_per_container", "hardware_id", "location_ids", "container_config", "registry_config"}
	case "extend":
		return []string{"duration_hours"}
	case "rename":
		return []string{"name"}
	case "update":
		return []string{"env_variables", "secret_env_variables", "entrypoint", "traffic_port", "image_url", "registry_username", "registry_secret", "args", "command"}
	default:
		return nil
	}
}
func ParseIoNetMutation(kind, id string, body json.RawMessage) (IoNetMutation, error) {
	out := IoNetMutation{Resource: "deployment:" + id, Method: http.MethodPatch, Endpoint: "/deployment/" + id}
	decode := func(v any) error {
		d := json.NewDecoder(bytes.NewReader(body))
		d.DisallowUnknownFields()
		if d.Decode(v) != nil {
			return ErrInvalid
		}
		return nil
	}
	var typed any
	if kind != "create" && !ValidIoNetID(id) {
		return out, ErrInvalid
	}
	switch kind {
	case "create":
		var in IoNetCreate
		if decode(&in) != nil || !ValidIoNetName(in.Name) || in.Hours < 1 || in.GPUs < 1 || in.Hardware < 1 || len(in.Locations) == 0 || len(in.Locations) > 100 || in.Container.ReplicaCount < 1 || in.Registry.Image == "" || len(in.Registry.Image) > 2048 || in.Container.Port < 0 || in.Container.Port > 65535 {
			return out, ErrInvalid
		}
		for _, location := range in.Locations {
			if location <= 0 {
				return out, ErrInvalid
			}
		}
		out.Resource = "create:" + in.Name
		out.Method = http.MethodPost
		out.Endpoint = "/deploy"
		typed = in
	case "extend":
		var in struct {
			Hours int `json:"duration_hours"`
		}
		if decode(&in) != nil || in.Hours < 1 {
			return out, ErrInvalid
		}
		out.Method = http.MethodPost
		out.Endpoint += "/extend"
		typed = in
	case "rename":
		var in struct {
			Name string `json:"name"`
		}
		if decode(&in) != nil || !ValidIoNetName(in.Name) {
			return out, ErrInvalid
		}
		out.Method = http.MethodPut
		out.Endpoint = "/clusters/" + id + "/update-name"
		typed = struct {
			Name string `json:"cluster_name"`
		}{in.Name}
	case "update":
		var in struct {
			Env        map[string]string `json:"env_variables,omitempty"`
			Secrets    map[string]string `json:"secret_env_variables,omitempty"`
			Entrypoint []string          `json:"entrypoint,omitempty"`
			Port       *int              `json:"traffic_port,omitempty"`
			Image      string            `json:"image_url,omitempty"`
			Username   string            `json:"registry_username,omitempty"`
			Secret     string            `json:"registry_secret,omitempty"`
			Args       []string          `json:"args,omitempty"`
			Command    string            `json:"command,omitempty"`
		}
		if decode(&in) != nil || in.Port != nil && (*in.Port < 1 || *in.Port > 65535) {
			return out, ErrInvalid
		}
		typed = in
	case "delete":
		out.Method = http.MethodDelete
		out.Body = nil
		return out, nil
	default:
		return out, ErrInvalid
	}
	canonical, err := json.Marshal(typed)
	if err != nil || len(canonical) > 131072 {
		return out, ErrInvalid
	}
	out.Body = canonical
	return out, nil
}
