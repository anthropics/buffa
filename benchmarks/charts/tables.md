### Binary decode

| Message | buffa | buffa (view) | buffa (lazy) | prost | prost (bytes) | protobuf-v4 | Go |
|---------|------:|------:|------:|------:|------:|------:|------:|
| ApiResponse | 594 | 742 (+25%) | 751 (+26%) | 580 (−2%) | 580 (−2%) | 438 (−26%) | 175 (−70%) |
| LogRecord | 472 | 1,105 (+134%) | 1,358 (+188%) | 480 (+2%) | 476 (+1%) | 522 (+11%) | 161 (−66%) |
| AnalyticsEvent | 125 | 196 (+57%) | 11,096 (+8802%) | 153 (+23%) | 130 (+5%) | 220 (+76%) | 57 (−54%) |
| GoogleMessage1 | 678 | 709 (+5%) | 1,266 (+87%) | 724 (+7%) | 671 (−1%) | 400 (−41%) | 263 (−61%) |
| MediaFrame | 10,330 | 36,390 (+252%) | 36,521 (+254%) | 6,039 (−42%) | 18,484 (+79%) | 10,531 (+2%) | 1,890 (−82%) |

### Binary encode

| Message | buffa | buffa (view) | buffa (lazy) | prost | prost (bytes) | protobuf-v4 | Go |
|---------|------:|------:|------:|------:|------:|------:|------:|
| ApiResponse | 1,987 | 1,931 (−3%) | 1,942 (−2%) | 1,991 (+0%) | — | 683 (−66%) | 384 (−81%) |
| LogRecord | 3,060 | 3,454 (+13%) | 3,635 (+19%) | 2,808 (−8%) | — | 983 (−68%) | 186 (−94%) |
| AnalyticsEvent | 409 | 427 (+4%) | 12,971 (+3068%) | 236 (−42%) | — | 313 (−24%) | 105 (−74%) |
| GoogleMessage1 | 2,170 | 2,140 (−1%) | 2,940 (+35%) | 1,834 (−15%) | — | 543 (−75%) | 232 (−89%) |
| MediaFrame | 25,616 | 27,015 (+5%) | 27,180 (+6%) | 25,703 (+0%) | — | 6,608 (−74%) | 2,423 (−91%) |

### Build + binary encode

| Message | buffa | buffa (view) |
|---------|------:|------:|
| ApiResponse | 643 | 1,243 (+93%) |
| LogRecord | 367 | 2,295 (+526%) |
| AnalyticsEvent | 267 | 804 (+201%) |
| GoogleMessage1 | 661 | 894 (+35%) |
| MediaFrame | 14,271 | 33,033 (+131%) |

### JSON encode

| Message | buffa | prost | Go |
|---------|------:|------:|------:|
| ApiResponse | 531 | 580 (+9%) | 70 (−87%) |
| LogRecord | 685 | 873 (+27%) | 85 (−88%) |
| AnalyticsEvent | 502 | 539 (+7%) | 33 (−94%) |
| GoogleMessage1 | 564 | 671 (+19%) | 73 (−87%) |
| MediaFrame | 700 | 934 (+33%) | 235 (−67%) |

### JSON decode

| Message | buffa | prost | Go |
|---------|------:|------:|------:|
| ApiResponse | 499 | 201 (−60%) | 40 (−92%) |
| LogRecord | 467 | 423 (−9%) | 63 (−87%) |
| AnalyticsEvent | 162 | 150 (−8%) | 25 (−85%) |
| GoogleMessage1 | 442 | 168 (−62%) | 41 (−91%) |
| MediaFrame | 1,212 | 1,214 (+0%) | 215 (−82%) |

### Reflection decode

| Message | generated | reflect | view |
|---------|------:|------:|------:|
| ApiResponse | 576 | 248 (−57%) | 740 (+28%) |
| LogRecord | 498 | 327 (−34%) | 1,096 (+120%) |
| AnalyticsEvent | 121 | 55 (−54%) | 192 (+59%) |
| GoogleMessage1 | 671 | 193 (−71%) | 685 (+2%) |

### Reflection encode

| Message | generated | reflect |
|---------|------:|------:|
| ApiResponse | 1,940 | 474 (−76%) |
| LogRecord | 3,030 | 875 (−71%) |
| AnalyticsEvent | 397 | 71 (−82%) |
| GoogleMessage1 | 2,115 | 223 (−89%) |

### Reflection read (decode + scan all fields)

| Message | vtable | bridge | dynamic |
|---------|------:|------:|------:|
| ApiResponse | 663 (+489%) | 113 | 188 (+67%) |
| LogRecord | 1,032 (+703%) | 129 | 279 (+117%) |
| AnalyticsEvent | 192 (+485%) | 33 | 54 (+66%) |
| GoogleMessage1 | 448 (+295%) | 113 | 139 (+22%) |
