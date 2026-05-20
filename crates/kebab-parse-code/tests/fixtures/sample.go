// sample.go
package chunk

import (
	"fmt"
	"strings"
)

const Version = "v1"

type MdHeadingV1Chunker struct {
	Name string
}

// ChunkDoc returns a stub list of strings.
func (m *MdHeadingV1Chunker) ChunkDoc(input string) []string {
	return []string{m.Name}
}

func (m MdHeadingV1Chunker) Name2() string {
	return m.Name
}

type Stringer interface {
	String() string
}

func Free(x int) int {
	return x + 1
}

func init() {
	fmt.Println(strings.ToUpper("init"))
}
