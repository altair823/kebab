// sample.java
package com.kebab.chunk;

import java.util.List;
import java.util.stream.Collectors;

/**
 * Heading-aware Markdown chunker.
 */
public class MdHeadingV1Chunker {
    private final String name;

    public MdHeadingV1Chunker(String name) {
        this.name = name;
    }

    public List<String> chunkDoc(String input) {
        return List.of(name, input);
    }

    public String getName() {
        return name;
    }

    public static class Builder {
        private String name;
        public Builder withName(String n) { this.name = n; return this; }
        public MdHeadingV1Chunker build() { return new MdHeadingV1Chunker(name); }
    }
}

interface Stringer {
    String asString();
}

enum Mode { DEFAULT, FAST }
