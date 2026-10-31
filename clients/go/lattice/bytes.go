// Package lattice is the experimental Go client for Lattice: the sync and change feed codecs.
// It is a port of clients/python/lattice_client and uses only the standard library.
package lattice

import (
	"encoding/binary"
)

// reader is a bounds-checked input cursor. A read past the end returns the error the codec
// gave it, so each codec reports truncation with its own error type.
type reader struct {
	buf   []byte
	pos   int
	short error
}

func newReader(buf []byte, short error) *reader {
	return &reader{buf: buf, short: short}
}

func (r *reader) remaining() int {
	return len(r.buf) - r.pos
}

// take returns a copy of the next n bytes, so a decoded value never aliases the input.
func (r *reader) take(n uint64) ([]byte, error) {
	if n > uint64(r.remaining()) {
		return nil, r.short
	}
	out := make([]byte, n)
	r.pos += copy(out, r.buf[r.pos:])
	return out, nil
}

func (r *reader) u8() (byte, error) {
	b, err := r.take(1)
	if err != nil {
		return 0, err
	}
	return b[0], nil
}

func (r *reader) u16be() (uint16, error) {
	b, err := r.take(2)
	if err != nil {
		return 0, err
	}
	return binary.BigEndian.Uint16(b), nil
}

func (r *reader) u16le() (uint16, error) {
	b, err := r.take(2)
	if err != nil {
		return 0, err
	}
	return binary.LittleEndian.Uint16(b), nil
}

func (r *reader) u32be() (uint32, error) {
	b, err := r.take(4)
	if err != nil {
		return 0, err
	}
	return binary.BigEndian.Uint32(b), nil
}

func (r *reader) u32le() (uint32, error) {
	b, err := r.take(4)
	if err != nil {
		return 0, err
	}
	return binary.LittleEndian.Uint32(b), nil
}

func (r *reader) u64be() (uint64, error) {
	b, err := r.take(8)
	if err != nil {
		return 0, err
	}
	return binary.BigEndian.Uint64(b), nil
}

func (r *reader) u64le() (uint64, error) {
	b, err := r.take(8)
	if err != nil {
		return 0, err
	}
	return binary.LittleEndian.Uint64(b), nil
}

// array16 reads a 16-byte value such as a change set id or a big endian u128.
func (r *reader) array16() ([16]byte, error) {
	var out [16]byte
	b, err := r.take(16)
	if err != nil {
		return out, err
	}
	copy(out[:], b)
	return out, nil
}

// hasControlChar reports whether s holds a Unicode control character (category Cc), the same set
// as Rust's char::is_control. Callers check UTF-8 validity first.
func hasControlChar(s string) bool {
	for _, c := range s {
		if c <= 0x1f || (c >= 0x7f && c <= 0x9f) {
			return true
		}
	}
	return false
}
