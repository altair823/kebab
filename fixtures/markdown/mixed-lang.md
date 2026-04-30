# Mixed Language Note

이 문서는 한국어와 영어가 섞여 있습니다. The body has both Korean
sentences and English sentences. lingua는 통계적 언어 감지기를 제공합니다.
This is to test that auto-detect picks one of `ko` or `en` deterministically
when no `lang:` field is present in the frontmatter.

본문은 첫 4 KB만 분석되지만, 짧은 문서에서도 잘 동작해야 합니다.
The detector should pick the dominant language across the sample window.
