package lattice

import (
	"bytes"
	"encoding/binary"
	"strconv"
)

// Change feed v1: cursor and event codecs (formats/change-feed-v1.md). A port of lattice_core::feed
// and of the Python client's feed.py, with the same bounds, canonical form and order of checks.

const (
	// MaxKeyLen is the largest entity key an event may carry, in bytes.
	MaxKeyLen = 4096
	// MaxValueLen is the largest value an upsert may carry, in bytes.
	MaxValueLen = 16 * 1024 * 1024
	// CursorLen is the encoded cursor size in bytes.
	CursorLen = 12

	feedVersion byte = 1
)

// FeedErrorKind names why an event could not be built or encoded.
type FeedErrorKind string

const (
	FeedZeroLSN         FeedErrorKind = "zero_lsn"
	FeedIndexOutOfRange FeedErrorKind = "index_out_of_range"
	FeedKeyTooLarge     FeedErrorKind = "key_too_large"
	FeedValueTooLarge   FeedErrorKind = "value_too_large"
	FeedInvalidValue    FeedErrorKind = "invalid_value"
)

// FeedError mirrors lattice_core::feed::FeedError. It carries no detail.
type FeedError struct {
	Kind FeedErrorKind
}

func (e *FeedError) Error() string { return string(e.Kind) }

// FeedDecodeErrorKind names why bytes are not a canonical feed value.
type FeedDecodeErrorKind string

const (
	FeedTruncated FeedDecodeErrorKind = "truncated"
	FeedVersion   FeedDecodeErrorKind = "version"
	FeedTrailing  FeedDecodeErrorKind = "trailing"
	FeedInvalid   FeedDecodeErrorKind = "invalid"
)

// FeedDecodeError mirrors lattice_core::feed::FeedDecodeError. Value is the version byte for
// version; Field names the field for invalid.
type FeedDecodeError struct {
	Kind  FeedDecodeErrorKind
	Value uint64
	Field string
}

func (e *FeedDecodeError) Error() string {
	switch {
	case e.Field != "":
		return string(e.Kind) + ": " + e.Field
	case e.Kind == FeedVersion:
		return string(e.Kind) + ": " + strconv.FormatUint(e.Value, 10)
	}
	return string(e.Kind)
}

var errFeedTruncated error = &FeedDecodeError{Kind: FeedTruncated}

func feedInvalid(field string) error {
	return &FeedDecodeError{Kind: FeedInvalid, Field: field}
}

// Cursor is a position in the feed, ordered by LSN and then Index. The origin is the zero value.
type Cursor struct {
	LSN   uint64
	Index uint32
}

// Origin is the cursor before every event.
var Origin = Cursor{}

// NewCursor is a cursor at event index of the change set logged at lsn. LSNs start at 1.
func NewCursor(lsn uint64, index uint32) (Cursor, error) {
	if lsn == 0 {
		return Cursor{}, &FeedError{Kind: FeedZeroLSN}
	}
	return Cursor{LSN: lsn, Index: index}, nil
}

// CompareCursors returns -1, 0 or 1 by (LSN, Index).
func CompareCursors(a, b Cursor) int {
	switch {
	case a.LSN < b.LSN:
		return -1
	case a.LSN > b.LSN:
		return 1
	case a.Index < b.Index:
		return -1
	case a.Index > b.Index:
		return 1
	}
	return 0
}

// CursorToBytes encodes a cursor as lsn u64 then index u32, both big endian, so byte order is
// cursor order.
func CursorToBytes(c Cursor) []byte {
	out := make([]byte, 0, CursorLen)
	out = binary.BigEndian.AppendUint64(out, c.LSN)
	return binary.BigEndian.AppendUint32(out, c.Index)
}

// CompareEncodedCursors compares two encoded cursors as unsigned strings. It agrees with
// CompareCursors on their decoded values.
func CompareEncodedCursors(a, b []byte) int {
	return bytes.Compare(a, b)
}

func readCursor(r *reader) (Cursor, error) {
	lsn, err := r.u64be()
	if err != nil {
		return Cursor{}, err
	}
	index, err := r.u32be()
	if err != nil {
		return Cursor{}, err
	}
	if lsn == 0 && index != 0 {
		return Cursor{}, feedInvalid("cursor")
	}
	return Cursor{LSN: lsn, Index: index}, nil
}

// CursorFromBytes decodes exactly CursorLen bytes.
func CursorFromBytes(buf []byte) (Cursor, error) {
	r := newReader(buf, errFeedTruncated)
	c, err := readCursor(r)
	if err != nil {
		return Cursor{}, err
	}
	if r.remaining() != 0 {
		return Cursor{}, &FeedDecodeError{Kind: FeedTrailing}
	}
	return c, nil
}

// Hlc is the commit timestamp of a change set.
type Hlc struct {
	WallMS  uint64
	Counter uint32
	Node    uint32
}

// TurnRef is the turn a write belonged to.
type TurnRef struct {
	Session uint64
	Turn    uint64
}

// ChangeOp says what happened to the entity: Delete or Upsert. Use the value types.
type ChangeOp interface {
	isChangeOp()
}

// Delete says the entity was deleted.
type Delete struct{}

// Upsert says the entity was created or replaced with Value.
type Upsert struct {
	Value []byte
}

func (Delete) isChangeOp() {}
func (Upsert) isChangeOp() {}

// ChangeEvent is one entity change of one committed change set, with its commit provenance.
type ChangeEvent struct {
	Cursor Cursor
	// SetLen is the number of events the change set produced.
	SetLen uint32
	// ChangeSet is the change set id, a big endian ULID.
	ChangeSet [16]byte
	HLC       Hlc
	// Author is who made the change, a big endian u128.
	Author [16]byte
	// Turn is nil when the write had no turn.
	Turn *TurnRef
	// Kind is the table or collection of the entity.
	Kind uint32
	Key  []byte
	Op   ChangeOp
}

func checkChange(ev ChangeEvent) error {
	if len(ev.Key) > MaxKeyLen {
		return &FeedError{Kind: FeedKeyTooLarge}
	}
	if up, ok := ev.Op.(Upsert); ok && len(up.Value) > MaxValueLen {
		return &FeedError{Kind: FeedValueTooLarge}
	}
	return nil
}

// EncodeEvent encodes an event to the v1 layout.
func EncodeEvent(ev ChangeEvent) ([]byte, error) {
	if ev.Cursor.LSN == 0 {
		return nil, &FeedError{Kind: FeedZeroLSN}
	}
	if ev.Cursor.Index >= ev.SetLen {
		return nil, &FeedError{Kind: FeedIndexOutOfRange}
	}
	if err := checkChange(ev); err != nil {
		return nil, err
	}
	if ev.Op == nil {
		return nil, &FeedError{Kind: FeedInvalidValue}
	}
	out := []byte{feedVersion}
	out = binary.BigEndian.AppendUint64(out, ev.Cursor.LSN)
	out = binary.BigEndian.AppendUint32(out, ev.Cursor.Index)
	out = binary.LittleEndian.AppendUint32(out, ev.SetLen)
	out = append(out, ev.ChangeSet[:]...)
	out = binary.LittleEndian.AppendUint64(out, ev.HLC.WallMS)
	out = binary.LittleEndian.AppendUint32(out, ev.HLC.Counter)
	out = binary.LittleEndian.AppendUint32(out, ev.HLC.Node)
	out = append(out, ev.Author[:]...)
	if ev.Turn == nil {
		out = append(out, 0)
	} else {
		out = append(out, 1)
		out = binary.LittleEndian.AppendUint64(out, ev.Turn.Session)
		out = binary.LittleEndian.AppendUint64(out, ev.Turn.Turn)
	}
	out = binary.LittleEndian.AppendUint32(out, ev.Kind)
	out = binary.LittleEndian.AppendUint16(out, uint16(len(ev.Key)))
	out = append(out, ev.Key...)
	switch op := ev.Op.(type) {
	case Delete:
		out = append(out, 0)
	case Upsert:
		out = append(out, 1)
		out = binary.LittleEndian.AppendUint32(out, uint32(len(op.Value)))
		out = append(out, op.Value...)
	default:
		return nil, &FeedError{Kind: FeedInvalidValue}
	}
	return out, nil
}

// DecodeEvent decodes the v1 layout. Only the bytes EncodeEvent produces are accepted.
func DecodeEvent(buf []byte) (ChangeEvent, error) {
	r := newReader(buf, errFeedTruncated)
	version, err := r.u8()
	if err != nil {
		return ChangeEvent{}, err
	}
	if version != feedVersion {
		return ChangeEvent{}, &FeedDecodeError{Kind: FeedVersion, Value: uint64(version)}
	}
	cursor, err := readCursor(r)
	if err != nil {
		return ChangeEvent{}, err
	}
	if cursor.LSN == 0 {
		return ChangeEvent{}, feedInvalid("event lsn")
	}
	setLen, err := r.u32le()
	if err != nil {
		return ChangeEvent{}, err
	}
	if cursor.Index >= setLen {
		return ChangeEvent{}, feedInvalid("event index")
	}
	ev := ChangeEvent{Cursor: cursor, SetLen: setLen}
	if ev.ChangeSet, err = r.array16(); err != nil {
		return ChangeEvent{}, err
	}
	if ev.HLC.WallMS, err = r.u64le(); err != nil {
		return ChangeEvent{}, err
	}
	if ev.HLC.Counter, err = r.u32le(); err != nil {
		return ChangeEvent{}, err
	}
	if ev.HLC.Node, err = r.u32le(); err != nil {
		return ChangeEvent{}, err
	}
	if ev.Author, err = r.array16(); err != nil {
		return ChangeEvent{}, err
	}
	flag, err := r.u8()
	if err != nil {
		return ChangeEvent{}, err
	}
	switch flag {
	case 0:
	case 1:
		var turn TurnRef
		if turn.Session, err = r.u64le(); err != nil {
			return ChangeEvent{}, err
		}
		if turn.Turn, err = r.u64le(); err != nil {
			return ChangeEvent{}, err
		}
		ev.Turn = &turn
	default:
		return ChangeEvent{}, feedInvalid("turn flag")
	}
	if ev.Kind, err = r.u32le(); err != nil {
		return ChangeEvent{}, err
	}
	keyLen, err := r.u16le()
	if err != nil {
		return ChangeEvent{}, err
	}
	if int(keyLen) > MaxKeyLen {
		return ChangeEvent{}, feedInvalid("key length")
	}
	if ev.Key, err = r.take(uint64(keyLen)); err != nil {
		return ChangeEvent{}, err
	}
	tag, err := r.u8()
	if err != nil {
		return ChangeEvent{}, err
	}
	switch tag {
	case 0:
		ev.Op = Delete{}
	case 1:
		length, err := r.u32le()
		if err != nil {
			return ChangeEvent{}, err
		}
		if length > MaxValueLen {
			return ChangeEvent{}, feedInvalid("value length")
		}
		// Checked against the bytes that remain before anything is copied.
		value, err := r.take(uint64(length))
		if err != nil {
			return ChangeEvent{}, err
		}
		ev.Op = Upsert{Value: value}
	default:
		return ChangeEvent{}, feedInvalid("operation")
	}
	if r.remaining() != 0 {
		return ChangeEvent{}, &FeedDecodeError{Kind: FeedTrailing}
	}
	return ev, nil
}
