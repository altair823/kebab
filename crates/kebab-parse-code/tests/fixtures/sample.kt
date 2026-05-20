// sample.kt
package com.kebab.chunk

import java.util.List

/**
 * Heading-aware Markdown chunker.
 */
class MdHeadingV1Chunker(val name: String) {
    fun chunkDoc(input: String): List<String> = listOf(name, input)

    fun getName(): String = name

    companion object {
        fun withName(n: String): MdHeadingV1Chunker = MdHeadingV1Chunker(n)
    }
}

interface Stringer {
    fun asString(): String
}

enum class Mode { DEFAULT, FAST }

fun freeFunction(x: Int): Int = x + 1

object Singleton {
    fun ping(): String = "pong"
}
