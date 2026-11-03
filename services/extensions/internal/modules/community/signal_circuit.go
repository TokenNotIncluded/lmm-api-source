// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"crypto/sha256"
	"encoding/binary"
	"errors"
	"time"
)

const signalRulesVersion = 2
const maxBoardSize = 64

var signalPorts = []int{1, 2, 4, 8}
var signalShapes = []int{3, 6, 12, 9, 5, 10}

func validSignalSize(size int) bool {
	switch size {
	case 5, 8, 12, 24, 48, maxBoardSize:
		return true
	}
	return false
}
func signalMaxMoves(size int) int {
	if size*size*16 < 400 {
		return 400
	}
	return size * size * 16
}

type signalCircuit struct {
	Tiles    []int `json:"tiles"`
	Solution []int `json:"-"`
	Route    []int `json:"-"`
}

func signalRotate(mask int) int { return ((mask << 1) & 15) | (mask >> 3) }
func signalNeighbor(index, port, size int) int {
	row, col := index/size, index%size
	switch port {
	case 1:
		if row > 0 {
			return index - size
		}
	case 2:
		if col < size-1 {
			return index + 1
		}
	case 4:
		if row < size-1 {
			return index + size
		}
	case 8:
		if col > 0 {
			return index - 1
		}
	}
	return -1
}
func signalWon(tiles []int, size int) bool {
	if !validSignalSize(size) || len(tiles) != size*size {
		return false
	}
	// A fixed bitset covers every supported board without a heap allocation
	// for each move checked by Replay. The traversal and cycle rule are unchanged.
	var seen [maxBoardSize * maxBoardSize / 64]uint64
	index, incoming := size/2*size, 8
	for tiles[index]&incoming != 0 {
		word, bit := index/64, uint64(1)<<uint(index%64)
		if seen[word]&bit != 0 {
			return false
		}
		seen[word] |= bit
		outgoing := 0
		for _, p := range signalPorts {
			if p != incoming && tiles[index]&p != 0 {
				outgoing = p
				break
			}
		}
		if outgoing == 0 {
			return false
		}
		if index == size/2*size+size-1 && outgoing == 2 {
			return true
		}
		index = signalNeighbor(index, outgoing, size)
		if index < 0 {
			return false
		}
		incoming = ((outgoing << 2) | (outgoing >> 2)) & 15
	}
	return false
}
func signalGenerate(seed uint32, size int) signalCircuit {
	if !validSignalSize(size) {
		return signalCircuit{}
	}
	state := seed
	random := func() float64 { state = state*1664525 + 1013904223; return float64(state) / 4294967296 }
	shuffled := func() []int {
		result := append([]int(nil), signalPorts...)
		for i := 3; i > 0; i-- {
			j := int(random() * float64(i+1))
			result[i], result[j] = result[j], result[i]
		}
		return result
	}
	type frame struct {
		index int
		ports []int
		next  int
	}
	start, finish := size/2*size, size/2*size+size-1
	visited := make([]bool, size*size)
	visited[start] = true
	stack := []frame{{start, shuffled(), 0}}
	var route []int
	for len(stack) > 0 {
		current := &stack[len(stack)-1]
		if current.index == finish {
			for _, item := range stack {
				route = append(route, item.index)
			}
			break
		}
		if current.next == 4 {
			stack = stack[:len(stack)-1]
			continue
		}
		next := signalNeighbor(current.index, current.ports[current.next], size)
		current.next++
		if next < 0 || visited[next] || (next == finish && len(stack) < 2*size-2) {
			continue
		}
		visited[next] = true
		stack = append(stack, frame{next, shuffled(), 0})
	}
	if len(route) == 0 {
		for row := size / 2; row >= 0; row-- {
			route = append(route, row*size)
		}
		for col := 1; col < size; col++ {
			route = append(route, col)
		}
		for row := 1; row <= size/2; row++ {
			route = append(route, row*size+size-1)
		}
	}
	solution := make([]int, size*size)
	for i := range solution {
		solution[i] = signalShapes[int(random()*6)]
	}
	for i, index := range route {
		before, after := 8, 2
		if i > 0 {
			for _, p := range signalPorts {
				if signalNeighbor(index, p, size) == route[i-1] {
					before = p
					break
				}
			}
		}
		if i < len(route)-1 {
			for _, p := range signalPorts {
				if signalNeighbor(index, p, size) == route[i+1] {
					after = p
					break
				}
			}
		}
		solution[index] = before | after
	}
	tiles := append([]int(nil), solution...)
	for i, mask := range tiles {
		turns := int(random() * 4)
		for j := 0; j < turns; j++ {
			mask = signalRotate(mask)
		}
		tiles[i] = mask
	}
	for signalWon(tiles, size) {
		tiles[start] = signalRotate(tiles[start])
	}
	return signalCircuit{tiles, solution, route}
}
func signalDailySeed(day string, size int) (uint32, error) {
	parsed, err := time.Parse("2006-01-02", day)
	if err != nil || parsed.Format("2006-01-02") != day || !validSignalSize(size) {
		return 0, errors.New("invalid challenge")
	}
	digest := sha256.Sum256([]byte("lmm-signal-v2:" + day))
	return binary.BigEndian.Uint32(digest[:4]), nil
}
func signalReplay(seed uint32, size int, actions []int, allowHints bool) (int, error) {
	if !validSignalSize(size) || len(actions) == 0 || len(actions) > signalMaxMoves(size) {
		return 0, errors.New("invalid move count")
	}
	c := signalGenerate(seed, size)
	tiles := append([]int(nil), c.Tiles...)
	hints := 0
	for _, action := range actions {
		if signalWon(tiles, size) {
			return 0, errors.New("moves after completion")
		}
		index := action
		if action == -1 && allowHints {
			index = -1
			for _, tile := range c.Route {
				if tiles[tile] != c.Solution[tile] {
					index = tile
					break
				}
			}
			hints++
		}
		if index < 0 || index >= size*size {
			return 0, errors.New("invalid tile or forbidden hint")
		}
		tiles[index] = signalRotate(tiles[index])
	}
	if !signalWon(tiles, size) {
		return 0, errors.New("circuit is not connected")
	}
	return hints, nil
}
