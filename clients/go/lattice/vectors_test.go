package lattice

import (
	"encoding/hex"
	"encoding/json"
	"errors"
	"math/big"
	"os"
	"strconv"
	"testing"
)

// The vectors the Rust side wrote. The path is relative to this package directory.
const vectorsPath = "../../../conformance/vectors.json"

type obj = map[string]any

type vectorFile struct {
	Format  string           `json:"format"`
	Version int              `json:"version"`
	Sync    map[string][]obj `json:"sync"`
	Feed    map[string][]obj `json:"feed"`
}

var vectors = loadVectors()

func loadVectors() *vectorFile {
	raw, err := os.ReadFile(vectorsPath)
	if err != nil {
		panic("conformance vectors: " + err.Error())
	}
	var v vectorFile
	if err := json.Unmarshal(raw, &v); err != nil {
		panic("conformance vectors: " + err.Error())
	}
	return &v
}

// rows returns the vectors under key and fails when there are none, so an emptied file cannot
// make a test pass by checking nothing.
func rows(t *testing.T, section map[string][]obj, key string) []obj {
	t.Helper()
	out := section[key]
	if len(out) == 0 {
		t.Fatalf("no vectors under %q", key)
	}
	return out
}

func str(t *testing.T, v any) string {
	t.Helper()
	s, ok := v.(string)
	if !ok {
		t.Fatalf("want a string, got %T", v)
	}
	return s
}

func field(t *testing.T, o obj, key string) any {
	t.Helper()
	v, ok := o[key]
	if !ok {
		t.Fatalf("vector has no %q", key)
	}
	return v
}

func obj0(t *testing.T, v any) obj {
	t.Helper()
	o, ok := v.(obj)
	if !ok {
		t.Fatalf("want an object, got %T", v)
	}
	return o
}

// num reads a JSON number that must be a whole number in range.
func num(t *testing.T, v any) uint64 {
	t.Helper()
	f, ok := v.(float64)
	if !ok || f < 0 || f != float64(uint64(f)) {
		t.Fatalf("want a whole number, got %v", v)
	}
	return uint64(f)
}

// decimal reads a 64-bit integer given as a decimal string, as the vector file does for values
// above 2^53. A plain JSON number is accepted too.
func decimal(t *testing.T, v any) uint64 {
	t.Helper()
	if s, ok := v.(string); ok {
		n, err := strconv.ParseUint(s, 10, 64)
		if err != nil {
			t.Fatalf("bad decimal %q: %v", s, err)
		}
		return n
	}
	return num(t, v)
}

// u128 reads a 128-bit integer given as a decimal string into 16 big endian bytes.
func u128(t *testing.T, v any) [16]byte {
	t.Helper()
	n, ok := new(big.Int).SetString(str(t, v), 10)
	if !ok || n.Sign() < 0 || n.BitLen() > 128 {
		t.Fatalf("bad u128 %v", v)
	}
	var out [16]byte
	n.FillBytes(out[:])
	return out
}

// hexBytes parses lowercase hex into a non-nil slice, so decoded and expected values compare equal
// with reflect.DeepEqual even when they are empty.
func hexBytes(t *testing.T, v any) []byte {
	t.Helper()
	s := str(t, v)
	out := make([]byte, len(s)/2)
	if _, err := hex.Decode(out, []byte(s)); err != nil {
		t.Fatalf("bad hex %q: %v", s, err)
	}
	return out
}

// bytesOf reads hex text, or {"fill":"ab","len":N} for N copies of one byte.
func bytesOf(t *testing.T, v any) []byte {
	t.Helper()
	if _, ok := v.(string); ok {
		return hexBytes(t, v)
	}
	o := obj0(t, v)
	fill := hexBytes(t, field(t, o, "fill"))
	if len(fill) != 1 {
		t.Fatalf("fill must be one byte")
	}
	n := num(t, field(t, o, "len"))
	out := make([]byte, n)
	for i := range out {
		out[i] = fill[0]
	}
	return out
}

// errParts reports the kind and detail of a codec error, as the vector file names them.
func errParts(t *testing.T, err error) (kind string, value uint64, field string) {
	t.Helper()
	var se *SyncError
	var fd *FeedDecodeError
	var fe *FeedError
	switch {
	case errors.As(err, &se):
		return string(se.Kind), se.Value, se.Field
	case errors.As(err, &fd):
		return string(fd.Kind), fd.Value, fd.Field
	case errors.As(err, &fe):
		return string(fe.Kind), 0, ""
	}
	t.Fatalf("not a codec error: %v", err)
	return "", 0, ""
}

// assertErrorMatches requires the kind and the detail the Rust error has. A value or a field is
// compared when the vector names one, and must be zero or empty when it does not.
func assertErrorMatches(t *testing.T, err error, want obj, label string) {
	t.Helper()
	if err == nil {
		t.Fatalf("%s: want error %v, got none", label, want)
	}
	kind, value, fieldName := errParts(t, err)
	if kind != str(t, field(t, want, "kind")) {
		t.Fatalf("%s: kind %q, want %q (%v)", label, kind, want["kind"], err)
	}
	if v, ok := want["value"]; ok {
		if value != num(t, v) {
			t.Fatalf("%s: value %d, want %v", label, value, v)
		}
	} else if want["field"] == nil && value != 0 {
		t.Fatalf("%s: unexpected value %d", label, value)
	}
	if f, ok := want["field"]; ok {
		if fieldName != str(t, f) {
			t.Fatalf("%s: field %q, want %q", label, fieldName, f)
		}
	} else if fieldName != "" {
		t.Fatalf("%s: unexpected field %q", label, fieldName)
	}
}

func messageFromJSON(t *testing.T, j obj) Message {
	t.Helper()
	switch kind := str(t, field(t, j, "type")); kind {
	case "hello":
		return Hello{
			MinVersion: uint16(num(t, field(t, j, "min_version"))),
			MaxVersion: uint16(num(t, field(t, j, "max_version"))),
			Agent:      str(t, field(t, j, "agent")),
		}
	case "welcome":
		return Welcome{
			Version:    uint16(num(t, field(t, j, "version"))),
			MaxPayload: uint32(num(t, field(t, j, "max_payload"))),
			Server:     str(t, field(t, j, "server")),
		}
	case "subscribe":
		return Subscribe{
			Sub:    uint32(num(t, field(t, j, "sub"))),
			Scope:  str(t, field(t, j, "scope")),
			Cursor: bytesOf(t, field(t, j, "cursor")),
		}
	case "events":
		var events []SyncEvent
		for _, e := range field(t, j, "events").([]any) {
			eo := obj0(t, e)
			events = append(events, SyncEvent{
				Cursor: bytesOf(t, field(t, eo, "cursor")),
				Data:   bytesOf(t, field(t, eo, "data")),
			})
		}
		return Events{Sub: uint32(num(t, field(t, j, "sub"))), Events: events}
	case "ack":
		return Ack{
			Sub:    uint32(num(t, field(t, j, "sub"))),
			Cursor: bytesOf(t, field(t, j, "cursor")),
		}
	case "error":
		return Error{
			Code:    ErrorCode(str(t, field(t, j, "code"))),
			Sub:     uint32(num(t, field(t, j, "sub"))),
			Message: str(t, field(t, j, "message")),
		}
	case "ping":
		return Ping{Nonce: decimal(t, field(t, j, "nonce"))}
	case "pong":
		return Pong{Nonce: decimal(t, field(t, j, "nonce"))}
	default:
		t.Fatalf("unknown message type in vectors: %s", kind)
	}
	return nil
}

func eventFromJSON(t *testing.T, j obj) ChangeEvent {
	t.Helper()
	cur := obj0(t, field(t, j, "cursor"))
	hlc := obj0(t, field(t, j, "hlc"))
	ev := ChangeEvent{
		Cursor: Cursor{LSN: decimal(t, field(t, cur, "lsn")), Index: uint32(num(t, field(t, cur, "index")))},
		SetLen: uint32(num(t, field(t, j, "set_len"))),
		HLC: Hlc{
			WallMS:  decimal(t, field(t, hlc, "wall_ms")),
			Counter: uint32(num(t, field(t, hlc, "counter"))),
			Node:    uint32(num(t, field(t, hlc, "node"))),
		},
		Author: u128(t, field(t, j, "author")),
		Kind:   uint32(num(t, field(t, j, "kind"))),
		Key:    bytesOf(t, field(t, j, "key")),
	}
	copy(ev.ChangeSet[:], bytesOf(t, field(t, j, "change_set")))
	if turn := field(t, j, "turn"); turn != nil {
		to := obj0(t, turn)
		ev.Turn = &TurnRef{Session: decimal(t, field(t, to, "session")), Turn: decimal(t, field(t, to, "turn"))}
	}
	op := obj0(t, field(t, j, "op"))
	switch str(t, field(t, op, "type")) {
	case "delete":
		ev.Op = Delete{}
	case "upsert":
		ev.Op = Upsert{Value: bytesOf(t, field(t, op, "value"))}
	default:
		t.Fatalf("unknown op in vectors")
	}
	return ev
}
