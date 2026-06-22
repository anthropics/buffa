### Binary decode

| Message | buffa | buffa (view) | buffa (lazy) | prost | prost (bytes) | protobuf-v4 | Go |
|---------|------:|------:|------:|------:|------:|------:|------:|
| ApiResponse | 582 | 752 (+29%) | 748 (+29%) | 555 (−5%) | 546 (−6%) | 430 (−26%) | 175 (−70%) |
| LogRecord | 543 | 1,113 (+105%) | 1,354 (+149%) | 482 (−11%) | 472 (−13%) | 555 (+2%) | 161 (−70%) |
| AnalyticsEvent | 125 | 194 (+54%) | 11,015 (+8680%) | 148 (+18%) | 129 (+3%) | 222 (+77%) | 57 (−55%) |
| GoogleMessage1 | 594 | 722 (+21%) | 1,253 (+111%) | 693 (+17%) | 661 (+11%) | 370 (−38%) | 263 (−56%) |
| MediaFrame | 10,549 | 36,486 (+246%) | 36,276 (+244%) | 6,113 (−42%) | 18,389 (+74%) | 11,045 (+5%) | 1,890 (−82%) |

### Binary encode

| Message | buffa | buffa (view) | buffa (lazy) | prost | prost (bytes) | protobuf-v4 | Go |
|---------|------:|------:|------:|------:|------:|------:|------:|
| ApiResponse | 1,956 | 1,942 (−1%) | 1,947 (−0%) | 1,956 (+0%) | — | 632 (−68%) | 384 (−80%) |
| LogRecord | 3,015 | 3,497 (+16%) | 3,601 (+19%) | 2,784 (−8%) | — | 1,071 (−64%) | 186 (−94%) |
| AnalyticsEvent | 411 | 431 (+5%) | 13,269 (+3128%) | 237 (−42%) | — | 309 (−25%) | 105 (−74%) |
| GoogleMessage1 | 2,157 | 2,123 (−2%) | 2,921 (+35%) | 1,815 (−16%) | — | 521 (−76%) | 232 (−89%) |
| MediaFrame | 25,841 | 27,159 (+5%) | 27,219 (+5%) | 25,655 (−1%) | — | 6,726 (−74%) | 2,423 (−91%) |

### Build + binary encode

| Message | buffa | buffa (view) |
|---------|------:|------:|
| ApiResponse | 645 | 1,223 (+90%) |
| LogRecord | 322 | 2,385 (+640%) |
| AnalyticsEvent | 262 | 819 (+212%) |
| GoogleMessage1 | 680 | 896 (+32%) |
| MediaFrame | 14,022 | 34,924 (+149%) |

### JSON encode

| Message | buffa | prost | Go |
|---------|------:|------:|------:|
| ApiResponse | 527 | 574 (+9%) | 70 (−87%) |
| LogRecord | 689 | 879 (+28%) | 85 (−88%) |
| AnalyticsEvent | 504 | 536 (+6%) | 33 (−94%) |
| GoogleMessage1 | 573 | 682 (+19%) | 73 (−87%) |
| MediaFrame | 704 | 936 (+33%) | 235 (−67%) |

### JSON decode

| Message | buffa | prost | Go |
|---------|------:|------:|------:|
| ApiResponse | 494 | 205 (−59%) | 40 (−92%) |
| LogRecord | 518 | 421 (−19%) | 63 (−88%) |
| AnalyticsEvent | 166 | 153 (−8%) | 25 (−85%) |
| GoogleMessage1 | 401 | 173 (−57%) | 41 (−90%) |
| MediaFrame | 1,224 | 1,231 (+1%) | 215 (−82%) |

### Reflection decode

| Message | generated | reflect | view |
|---------|------:|------:|------:|
| ApiResponse | 588 | 246 (−58%) | 729 (+24%) |
| LogRecord | 566 | 314 (−44%) | 1,057 (+87%) |
| AnalyticsEvent | 128 | 54 (−58%) | 193 (+51%) |
| GoogleMessage1 | 667 | 190 (−71%) | 681 (+2%) |

### Reflection encode

| Message | generated | reflect |
|---------|------:|------:|
| ApiResponse | 1,951 | 477 (−76%) |
| LogRecord | 3,001 | 870 (−71%) |
| AnalyticsEvent | 398 | 71 (−82%) |
| GoogleMessage1 | 2,112 | 221 (−90%) |

### Reflection read (decode + scan all fields)

| Message | vtable | bridge | dynamic |
|---------|------:|------:|------:|
| ApiResponse | 658 (+484%) | 113 | 186 (+65%) |
| LogRecord | 1,015 (+667%) | 132 | 277 (+110%) |
| AnalyticsEvent | 193 (+489%) | 33 | 54 (+65%) |
| GoogleMessage1 | 445 (+291%) | 114 | 139 (+22%) |
