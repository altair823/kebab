# Alpha

Alpha intro paragraph one. This first paragraph in the alpha section gives a brief overview of what is to follow and serves as the lead-in for the subsequent material covered under the alpha heading.

Alpha intro paragraph two. The second paragraph extends the discussion with additional sentences, padding out the paragraph so that paragraph-level chunk splitting actually has multiple candidates to consider when deciding where to slice the content stream.

## Alpha Sub

Some prose under the alpha sub-heading. The nested heading should still be respected as a chunk boundary distinct from the parent alpha heading.

```rust
// A code block long enough to easily clear any reasonable target_tokens
// so the never-split-code-block rule is exercised by this fixture. The
// rest of the function body is intentional filler: line after line of
// content that, were the chunker permitted to split it, would exceed
// the target threshold and force a break in the middle of the snippet.
fn long_code_example_one() {
    let mut numbers = Vec::new();
    for i in 0..10 {
        numbers.push(i * 2);
    }
    let mut total = 0_i64;
    for n in &numbers {
        total += *n as i64;
    }
    println!("total = {total}");
}

fn long_code_example_two() {
    let words = ["alpha", "beta", "gamma", "delta", "epsilon"];
    for w in words.iter() {
        if w.starts_with('a') {
            println!("starts with a: {w}");
        } else if w.starts_with('b') {
            println!("starts with b: {w}");
        } else if w.starts_with('g') {
            println!("starts with g: {w}");
        } else {
            println!("other: {w}");
        }
    }
}

fn long_code_example_three() {
    let mut buf = String::new();
    for ch in "lorem ipsum dolor sit amet".chars() {
        if ch.is_ascii_alphabetic() {
            buf.push(ch.to_ascii_uppercase());
        }
    }
    println!("buf = {buf}");
}
```

# Beta

Beta paragraph one. The beta section opens with an introductory paragraph that sets up the table appearing further down.

| name  | kind   | note         |
|-------|--------|--------------|
| one   | small  | first row    |
| two   | medium | second row   |
| three | large  | third row    |
| four  | huge   | fourth row   |

Beta closing paragraph. After the table we have one more paragraph of prose that anchors the end of the beta section before we move on to gamma.

# Gamma

Gamma paragraph one. The gamma section is intentionally long to exercise the paragraph-level split with overlap rule when chunking under a single heading without any nested sub-headings to break things up further.

Gamma paragraph two. We continue accumulating prose so that the running token estimator climbs steadily and eventually trips the target_tokens threshold, forcing the chunker to emit a chunk and seed the next chunk with overlap from the prior tail.

Gamma paragraph three. Yet another paragraph under the gamma heading, padded with words to ensure the byte count clears the threshold and the splitting behaviour shows up unambiguously in the snapshot output.
