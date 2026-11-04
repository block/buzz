# 다국어 코드와 번역 자료 출처

원본 저장소와 그 Apache-2.0 LICENSE를 유지합니다. 이 브랜치는 아래 공개 기여에서 필요한 코드를 선택적으로 이식했습니다. 전체 패치를 cherry-pick한 것처럼 이력을 표시하지 않습니다.

| 자료 | 고정 출처 | 사용 내용 |
|---|---|---|
| Buzz 0.5.28 | `block/buzz` 태그 `desktop-v0.5.28`, `f87c10895ea35a9a2cea629fef1d7532a3fecbc5` | 데스크톱 앱 기준 소스 |
| i18n PR #5087 / #5152 | `1ad0bb60198ad33128c70a90a0b822b5daa9d550` (Sam Wong / hyperfocusam) | 언어 자원, i18n 기반 및 언어 선택 설계 |
| 시스템 언어 처리 | `b4560ace4d99ba70dcfdcfed842415df65418d11` (李亦真 / liyizhen) | 시스템 언어 선택 구조 |
| 공개 한국어 패치 | `Gwani-28/buzz`, `korean-ui-patch`, `0a1e20470327270e572b2495eddb55bd726951f6`, `team-patches/40-korean.patch` | 원문/한국어 문자열 2,446개와 패턴 321개의 리터럴 데이터 |

한국어 패치의 DOM MutationObserver, 문서 감시, 텍스트 노드 덮어쓰기 코드는 가져오지 않았습니다. TypeScript AST로 허용된 리터럴만 읽고, 현재 태그의 명시적인 UI 소스에 i18n 호출을 적용했습니다. 버전 차이로 새로 생긴 문구와 한국어 어순, 인라인 링크 처리, 데이터 격리 및 네이티브 메뉴 연결은 이 개발 브랜치에서 보완했습니다.

- 원본: https://github.com/block/buzz
- 공개 i18n PR: https://github.com/block/buzz/pull/5152
- 선행 PR: https://github.com/block/buzz/pull/5087
- 한국어 카탈로그: https://github.com/Gwani-28/buzz/tree/0a1e20470327270e572b2495eddb55bd726951f6/team-patches

각 자료의 공개 상태는 해당 시점의 소스 선택 근거입니다. maintainer 승인이나 병합이 확정되었다는 뜻은 아닙니다. 향후 upstream 기여 시 원 기여자의 저작자 표시와 기여 정책을 다시 확인합니다.
