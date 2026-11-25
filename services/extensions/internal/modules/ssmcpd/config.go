// Package ssmcpd describes the operator-configured SSMCP account service.
// No relay control credential or Core financial authority enters this plugin.
package ssmcpd

import (
	"crypto/subtle"
	"errors"
	"net/url"
	"regexp"
	"strconv"
)

type Config struct {
	Commerce      *CommerceConfig `toml:"commerce"`
	Metering      *InternalConfig `toml:"metering"`
	Mode          string          `toml:"mode"`
	PublicOrigin  string          `toml:"public_origin"`
	TrafficPortal bool            `toml:"traffic_portal"`
}

type Service struct {
	commerce       *trafficCommerce
	meter          *internalClient
	Mode           string `json:"mode"`
	PublicOrigin   string `json:"public_origin"`
	ConnectCommand string `json:"connect_command"`
	TrafficPortal  string `json:"traffic_portal,omitempty"`
}

var publicHost = regexp.MustCompile(`^[a-zA-Z0-9]([a-zA-Z0-9.-]*[a-zA-Z0-9])?(:[0-9]{1,5})?$`)

func New(config Config) (*Service, error) {
	origin, err := url.Parse(config.PublicOrigin)
	if err != nil || origin.Scheme != "https" || origin.Hostname() == "" ||
		origin.User != nil || origin.Path != "" || origin.RawQuery != "" || origin.Fragment != "" ||
		config.PublicOrigin != "https://"+origin.Host || !publicHost.MatchString(origin.Host) {
		return nil, errors.New("ssmcpd requires an HTTPS origin with no credentials, path, query or fragment")
	}
	if port := origin.Port(); port != "" {
		n, err := strconv.ParseUint(port, 10, 16)
		if err != nil || n == 0 {
			return nil, errors.New("invalid ssmcpd port")
		}
	}
	if (config.Mode != "relay" && config.Mode != "standard") || (config.TrafficPortal && config.Mode != "standard") {
		return nil, errors.New("ssmcpd requires relay or standard mode; the account traffic portal requires standard mode")
	}
	s := &Service{Mode: config.Mode, PublicOrigin: config.PublicOrigin, ConnectCommand: "ssmcp username@" + origin.Host}
	if config.TrafficPortal {
		s.TrafficPortal = config.PublicOrigin + "/account/traffic"
	}
	if config.Metering != nil {
		if config.Mode != "relay" {
			return nil, errors.New("external principal metering requires relay mode")
		}
		s.meter, err = newInternalClient(*config.Metering)
		if err != nil {
			return nil, err
		}
	}
	return s, nil
}

// Check that a relay credential cannot be substituted for a host or Core credential.
func (s *Service) CheckCredentials(tokens ...[]byte) error {
	clients := []*internalClient{s.meter}
	if s.commerce != nil {
		clients = append(clients, s.commerce.grants)
	}
	for _, client := range clients {
		if client == nil {
			continue
		}
		for _, token := range tokens {
			if subtle.ConstantTimeCompare([]byte(client.token), token) == 1 {
				return errors.New("ssmcpd requires a dedicated service credential")
			}
		}
	}
	return nil
}
