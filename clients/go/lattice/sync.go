package lattice

import (
	"encoding/binary"
	"fmt"
	"unicode/utf8"
)

// Sync protocol v1 codec (formats/sync-protocol-v1.md). A port of lattice_api::sync and of the
// Python client's sync.py: the same bounds, the same canonical encodings and the same order of
// checks, so the same bytes fail with the same error.

const (
	VersionMin uint16 = 1
	VersionMax uint16 = 1
	// HeaderLen is the number of bytes before the payload: a type byte and a u32 payload length.
	HeaderLen = 5
	// MaxPayload is the largest payload the codec accepts or produces.
	MaxPayload = 1 << 20
	// MinNegotiatedPayload is the smallest payload limit a Welcome may announce.
	MinNegotiatedPayload = 1024
	MaxCursorLen         = 64
	MaxScopeLen          = 512
	MaxNameLen           = 64
	MaxErrorMessageLen   = 256
	MaxEventsPerFrame    = 256
)

const (
	typeHello     byte = 0x01
	typeWelcome   byte = 0x02
	typeSubscribe byte = 0x03
	typeEvents    byte = 0x04
	typeAck       byte = 0x05
	typeError     byte = 0x06
	typePing      byte = 0x07
	typePong      byte = 0x08
)

// SyncErrorKind names why a frame could not be decoded or encoded.
type SyncErrorKind string

const (
	ErrBadLength    SyncErrorKind = "bad_length"
	ErrUnknownType  SyncErrorKind = "unknown_type"
	ErrTruncated    SyncErrorKind = "truncated"
	ErrTrailing     SyncErrorKind = "trailing"
	ErrNotUTF8      SyncErrorKind = "not_utf8"
	ErrTooLarge     SyncErrorKind = "too_large"
	ErrInvalidField SyncErrorKind = "invalid_field"
)

// SyncError mirrors lattice_api::sync::SyncError. Value carries the length for bad_length and the
// type byte for unknown_type; Field names the field for invalid_field. The other kinds carry neither.
type SyncError struct {
	Kind  SyncErrorKind
	Value uint64
	Field string
}

func (e *SyncError) Error() string {
	switch {
	case e.Field != "":
		return fmt.Sprintf("%s: %s", e.Kind, e.Field)
	case e.Kind == ErrBadLength || e.Kind == ErrUnknownType:
		return fmt.Sprintf("%s: %d", e.Kind, e.Value)
	}
	return string(e.Kind)
}

var errSyncTruncated error = &SyncError{Kind: ErrTruncated}

func syncInvalid(field string) error {
	return &SyncError{Kind: ErrInvalidField, Field: field}
}

// ErrorCode is the code in an Error message.
type ErrorCode string

const (
	ErrorUnsupportedVersion   ErrorCode = "unsupported_version"
	ErrorMalformed            ErrorCode = "malformed"
	ErrorUnknownSubscription  ErrorCode = "unknown_subscription"
	ErrorInvalidScope         ErrorCode = "invalid_scope"
	ErrorInvalidCursor        ErrorCode = "invalid_cursor"
	ErrorTooManySubscriptions ErrorCode = "too_many_subscriptions"
	ErrorInternal             ErrorCode = "internal"
)

// errorCodes lists the codes by wire number: element n-1 is number n.
var errorCodes = [...]ErrorCode{
	ErrorUnsupportedVersion,
	ErrorMalformed,
	ErrorUnknownSubscription,
	ErrorInvalidScope,
	ErrorInvalidCursor,
	ErrorTooManySubscriptions,
	ErrorInternal,
}

// ErrorCodeNumber is the wire number of a code. It fails for a name the protocol does not define.
func ErrorCodeNumber(code ErrorCode) (uint16, error) {
	for i, c := range errorCodes {
		if c == code {
			return uint16(i + 1), nil
		}
	}
	return 0, syncInvalid("error code")
}

// ErrorCodeFromNumber is the code for a wire number, or false if the protocol does not define it.
func ErrorCodeFromNumber(n uint16) (ErrorCode, bool) {
	if n >= 1 && int(n) <= len(errorCodes) {
		return errorCodes[n-1], true
	}
	return "", false
}

// Message is a sync protocol message. The method is unexported so only this package defines them.
type Message interface {
	isSyncMessage()
}

// SyncEvent is one change delivered to a subscription: the position after it and an opaque body.
type SyncEvent struct {
	Cursor []byte
	Data   []byte
}

type Hello struct {
	MinVersion uint16
	MaxVersion uint16
	Agent      string
}

type Welcome struct {
	Version    uint16
	MaxPayload uint32
	Server     string
}

type Subscribe struct {
	Sub    uint32
	Scope  string
	Cursor []byte
}

type Events struct {
	Sub    uint32
	Events []SyncEvent
}

type Ack struct {
	Sub    uint32
	Cursor []byte
}

type Error struct {
	Code    ErrorCode
	Sub     uint32
	Message string
}

type Ping struct {
	Nonce uint64
}

type Pong struct {
	Nonce uint64
}

func (Hello) isSyncMessage()     {}
func (Welcome) isSyncMessage()   {}
func (Subscribe) isSyncMessage() {}
func (Events) isSyncMessage()    {}
func (Ack) isSyncMessage()       {}
func (Error) isSyncMessage()     {}
func (Ping) isSyncMessage()      {}
func (Pong) isSyncMessage()      {}

// Negotiation is the result of Negotiate: Version when Error is empty, otherwise Error is
// "malformed" or "unsupported_version".
type Negotiation struct {
	Version uint16
	Error   string
}

// Negotiate picks the highest version both sides speak. The client offers minimum..=maximum.
func Negotiate(minimum, maximum uint16) Negotiation {
	if minimum < VersionMin || minimum > maximum {
		return Negotiation{Error: "malformed"}
	}
	if minimum > VersionMax || maximum < VersionMin {
		return Negotiation{Error: "unsupported_version"}
	}
	if maximum < VersionMax {
		return Negotiation{Version: maximum}
	}
	return Negotiation{Version: VersionMax}
}

func checkVersions(minimum, maximum uint16) error {
	if minimum < VersionMin || minimum > maximum {
		return syncInvalid("version range")
	}
	return nil
}

// checkWelcome refuses a version other than the one this codec speaks, rather than guessing.
func checkWelcome(version uint16, maxPayload uint32) error {
	if version < VersionMin || version > VersionMax {
		return syncInvalid("version")
	}
	if maxPayload < MinNegotiatedPayload || maxPayload > MaxPayload {
		return syncInvalid("max payload")
	}
	return nil
}

func checkSub(sub uint32) error {
	if sub == 0 {
		return syncInvalid("subscription id")
	}
	return nil
}

func checkText(s string, minLen, maxLen int, what string) error {
	if len(s) < minLen || len(s) > maxLen || hasControlChar(s) {
		return syncInvalid(what)
	}
	return nil
}

func appendText(b []byte, s string, minLen, maxLen int, what string) ([]byte, error) {
	if !utf8.ValidString(s) {
		return nil, &SyncError{Kind: ErrNotUTF8}
	}
	if err := checkText(s, minLen, maxLen, what); err != nil {
		return nil, err
	}
	b = binary.BigEndian.AppendUint16(b, uint16(len(s)))
	return append(b, s...), nil
}

func appendCursor(b []byte, c []byte, minLen int) ([]byte, error) {
	if len(c) < minLen || len(c) > MaxCursorLen {
		return nil, syncInvalid("cursor")
	}
	b = binary.BigEndian.AppendUint16(b, uint16(len(c)))
	return append(b, c...), nil
}

// Encode returns the frame for msg. It refuses whatever Decode would refuse, so a message that
// encodes always decodes to itself.
func Encode(msg Message) ([]byte, error) {
	var (
		ty      byte
		payload []byte
		err     error
	)
	switch m := msg.(type) {
	case Hello:
		if err = checkVersions(m.MinVersion, m.MaxVersion); err != nil {
			return nil, err
		}
		payload = binary.BigEndian.AppendUint16(payload, m.MinVersion)
		payload = binary.BigEndian.AppendUint16(payload, m.MaxVersion)
		if payload, err = appendText(payload, m.Agent, 1, MaxNameLen, "agent"); err != nil {
			return nil, err
		}
		ty = typeHello
	case Welcome:
		if err = checkWelcome(m.Version, m.MaxPayload); err != nil {
			return nil, err
		}
		payload = binary.BigEndian.AppendUint16(payload, m.Version)
		payload = binary.BigEndian.AppendUint32(payload, m.MaxPayload)
		if payload, err = appendText(payload, m.Server, 1, MaxNameLen, "server"); err != nil {
			return nil, err
		}
		ty = typeWelcome
	case Subscribe:
		if err = checkSub(m.Sub); err != nil {
			return nil, err
		}
		payload = binary.BigEndian.AppendUint32(payload, m.Sub)
		if payload, err = appendText(payload, m.Scope, 1, MaxScopeLen, "scope"); err != nil {
			return nil, err
		}
		if payload, err = appendCursor(payload, m.Cursor, 0); err != nil {
			return nil, err
		}
		ty = typeSubscribe
	case Events:
		if err = checkSub(m.Sub); err != nil {
			return nil, err
		}
		if len(m.Events) < 1 || len(m.Events) > MaxEventsPerFrame {
			return nil, syncInvalid("event count")
		}
		payload = binary.BigEndian.AppendUint32(payload, m.Sub)
		payload = binary.BigEndian.AppendUint16(payload, uint16(len(m.Events)))
		for _, e := range m.Events {
			if payload, err = appendCursor(payload, e.Cursor, 1); err != nil {
				return nil, err
			}
			if len(payload)+4+len(e.Data) > MaxPayload {
				return nil, &SyncError{Kind: ErrTooLarge}
			}
			payload = binary.BigEndian.AppendUint32(payload, uint32(len(e.Data)))
			payload = append(payload, e.Data...)
		}
		ty = typeEvents
	case Ack:
		if err = checkSub(m.Sub); err != nil {
			return nil, err
		}
		payload = binary.BigEndian.AppendUint32(payload, m.Sub)
		if payload, err = appendCursor(payload, m.Cursor, 1); err != nil {
			return nil, err
		}
		ty = typeAck
	case Error:
		code, err := ErrorCodeNumber(m.Code)
		if err != nil {
			return nil, err
		}
		payload = binary.BigEndian.AppendUint16(payload, code)
		payload = binary.BigEndian.AppendUint32(payload, m.Sub)
		if payload, err = appendText(payload, m.Message, 0, MaxErrorMessageLen, "message"); err != nil {
			return nil, err
		}
		ty = typeError
	case Ping:
		payload = binary.BigEndian.AppendUint64(payload, m.Nonce)
		ty = typePing
	case Pong:
		payload = binary.BigEndian.AppendUint64(payload, m.Nonce)
		ty = typePong
	default:
		return nil, &SyncError{Kind: ErrUnknownType}
	}
	if len(payload) > MaxPayload {
		return nil, &SyncError{Kind: ErrTooLarge}
	}
	out := make([]byte, 0, HeaderLen+len(payload))
	out = append(out, ty)
	out = binary.BigEndian.AppendUint32(out, uint32(len(payload)))
	return append(out, payload...), nil
}

// Decoded is one decoded frame and the number of bytes it used.
type Decoded struct {
	Message Message
	Length  int
}

// Decode decodes one frame from the front of buf with the default limit, MaxPayload. It returns
// nil and no error when buf holds only part of a frame.
func Decode(buf []byte) (*Decoded, error) {
	return DecodeWithLimit(buf, MaxPayload)
}

// DecodeWithLimit is Decode with a payload limit negotiated lower than MaxPayload. A higher limit
// counts as MaxPayload. The type and the length are judged as soon as the header is present, so
// a peer cannot make the reader wait for a payload it will refuse.
func DecodeWithLimit(buf []byte, limit uint64) (*Decoded, error) {
	if len(buf) == 0 {
		return nil, nil
	}
	ty := buf[0]
	if ty < typeHello || ty > typePong {
		return nil, &SyncError{Kind: ErrUnknownType, Value: uint64(ty)}
	}
	if len(buf) < HeaderLen {
		return nil, nil
	}
	length := uint64(binary.BigEndian.Uint32(buf[1:HeaderLen]))
	if length > min(limit, MaxPayload) {
		return nil, &SyncError{Kind: ErrBadLength, Value: length}
	}
	end := HeaderLen + int(length)
	if len(buf) < end {
		return nil, nil
	}
	msg, err := decodePayload(ty, buf[HeaderLen:end])
	if err != nil {
		return nil, err
	}
	return &Decoded{Message: msg, Length: end}, nil
}

func readText(r *reader, minLen, maxLen int, what string) (string, error) {
	n, err := r.u16be()
	if err != nil {
		return "", err
	}
	if int(n) < minLen || int(n) > maxLen {
		return "", syncInvalid(what)
	}
	raw, err := r.take(uint64(n))
	if err != nil {
		return "", err
	}
	if !utf8.Valid(raw) {
		return "", &SyncError{Kind: ErrNotUTF8}
	}
	s := string(raw)
	if err := checkText(s, minLen, maxLen, what); err != nil {
		return "", err
	}
	return s, nil
}

func readSyncCursor(r *reader, minLen int) ([]byte, error) {
	n, err := r.u16be()
	if err != nil {
		return nil, err
	}
	if int(n) < minLen || int(n) > MaxCursorLen {
		return nil, syncInvalid("cursor")
	}
	return r.take(uint64(n))
}

func decodePayload(ty byte, payload []byte) (Message, error) {
	r := newReader(payload, errSyncTruncated)
	var (
		msg Message
		err error
	)
	switch ty {
	case typeHello:
		msg, err = decodeHello(r)
	case typeWelcome:
		msg, err = decodeWelcome(r)
	case typeSubscribe:
		msg, err = decodeSubscribe(r)
	case typeEvents:
		msg, err = decodeEvents(r)
	case typeAck:
		msg, err = decodeAck(r)
	case typeError:
		msg, err = decodeError(r)
	case typePing:
		msg, err = decodePing(r)
	case typePong:
		msg, err = decodePong(r)
	default:
		return nil, &SyncError{Kind: ErrUnknownType, Value: uint64(ty)}
	}
	if err != nil {
		return nil, err
	}
	if r.remaining() != 0 {
		return nil, &SyncError{Kind: ErrTrailing}
	}
	return msg, nil
}

func decodeHello(r *reader) (Message, error) {
	minVersion, err := r.u16be()
	if err != nil {
		return nil, err
	}
	maxVersion, err := r.u16be()
	if err != nil {
		return nil, err
	}
	if err := checkVersions(minVersion, maxVersion); err != nil {
		return nil, err
	}
	agent, err := readText(r, 1, MaxNameLen, "agent")
	if err != nil {
		return nil, err
	}
	return Hello{MinVersion: minVersion, MaxVersion: maxVersion, Agent: agent}, nil
}

func decodeWelcome(r *reader) (Message, error) {
	version, err := r.u16be()
	if err != nil {
		return nil, err
	}
	maxPayload, err := r.u32be()
	if err != nil {
		return nil, err
	}
	if err := checkWelcome(version, maxPayload); err != nil {
		return nil, err
	}
	server, err := readText(r, 1, MaxNameLen, "server")
	if err != nil {
		return nil, err
	}
	return Welcome{Version: version, MaxPayload: maxPayload, Server: server}, nil
}

func decodeSubscribe(r *reader) (Message, error) {
	sub, err := r.u32be()
	if err != nil {
		return nil, err
	}
	if err := checkSub(sub); err != nil {
		return nil, err
	}
	scope, err := readText(r, 1, MaxScopeLen, "scope")
	if err != nil {
		return nil, err
	}
	cursor, err := readSyncCursor(r, 0)
	if err != nil {
		return nil, err
	}
	return Subscribe{Sub: sub, Scope: scope, Cursor: cursor}, nil
}

func decodeEvents(r *reader) (Message, error) {
	sub, err := r.u32be()
	if err != nil {
		return nil, err
	}
	if err := checkSub(sub); err != nil {
		return nil, err
	}
	count, err := r.u16be()
	if err != nil {
		return nil, err
	}
	if count == 0 || int(count) > MaxEventsPerFrame {
		return nil, syncInvalid("event count")
	}
	events := make([]SyncEvent, 0, count)
	for range count {
		cursor, err := readSyncCursor(r, 1)
		if err != nil {
			return nil, err
		}
		n, err := r.u32be()
		if err != nil {
			return nil, err
		}
		data, err := r.take(uint64(n))
		if err != nil {
			return nil, err
		}
		events = append(events, SyncEvent{Cursor: cursor, Data: data})
	}
	return Events{Sub: sub, Events: events}, nil
}

func decodeAck(r *reader) (Message, error) {
	sub, err := r.u32be()
	if err != nil {
		return nil, err
	}
	if err := checkSub(sub); err != nil {
		return nil, err
	}
	cursor, err := readSyncCursor(r, 1)
	if err != nil {
		return nil, err
	}
	return Ack{Sub: sub, Cursor: cursor}, nil
}

func decodeError(r *reader) (Message, error) {
	number, err := r.u16be()
	if err != nil {
		return nil, err
	}
	code, ok := ErrorCodeFromNumber(number)
	if !ok {
		return nil, syncInvalid("error code")
	}
	sub, err := r.u32be()
	if err != nil {
		return nil, err
	}
	message, err := readText(r, 0, MaxErrorMessageLen, "message")
	if err != nil {
		return nil, err
	}
	return Error{Code: code, Sub: sub, Message: message}, nil
}

func decodePing(r *reader) (Message, error) {
	nonce, err := r.u64be()
	if err != nil {
		return nil, err
	}
	return Ping{Nonce: nonce}, nil
}

func decodePong(r *reader) (Message, error) {
	nonce, err := r.u64be()
	if err != nil {
		return nil, err
	}
	return Pong{Nonce: nonce}, nil
}
