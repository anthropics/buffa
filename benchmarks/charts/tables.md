### Binary decode

| Message | buffa | buffa (view) | buffa (lazy) | prost | prost (bytes) | protobuf-v4 | Go |
|---------|------:|------:|------:|------:|------:|------:|------:|
| ApiResponse | 598 | 911 (+52%) | 933 (+56%) | 550 (−8%) | 546 (−9%) | 430 (−28%) | 175 (−71%) |
| LogRecord | 562 | 1,351 (+141%) | 1,685 (+200%) | 481 (−14%) | 477 (−15%) | 555 (−1%) | 161 (−71%) |
| AnalyticsEvent | 136 | 225 (+65%) | 11,624 (+8421%) | 148 (+9%) | 129 (−5%) | 222 (+63%) | 57 (−58%) |
| GoogleMessage1 | 646 | 767 (+19%) | 1,419 (+120%) | 698 (+8%) | 669 (+4%) | 373 (−42%) | 263 (−59%) |
| MediaFrame | 10,672 | 41,474 (+289%) | 41,489 (+289%) | 6,002 (−44%) | 18,432 (+73%) | 11,005 (+3%) | 1,890 (−82%) |

### Binary encode

| Message | buffa | buffa (view) | buffa (lazy) | prost | prost (bytes) | protobuf-v4 | Go |
|---------|------:|------:|------:|------:|------:|------:|------:|
| ApiResponse | 1,956 | 1,948 (−0%) | 1,910 (−2%) | 1,964 (+0%) | — | 639 (−67%) | 384 (−80%) |
| LogRecord | 2,997 | 3,501 (+17%) | 3,612 (+21%) | 2,758 (−8%) | — | 1,067 (−64%) | 186 (−94%) |
| AnalyticsEvent | 408 | 428 (+5%) | 12,852 (+3049%) | 238 (−42%) | — | 307 (−25%) | 105 (−74%) |
| GoogleMessage1 | 2,145 | 2,134 (−1%) | 2,912 (+36%) | 1,816 (−15%) | — | 522 (−76%) | 232 (−89%) |
| MediaFrame | 25,919 | 27,431 (+6%) | 26,939 (+4%) | 25,402 (−2%) | — | 6,671 (−74%) | 2,423 (−91%) |

### Build + binary encode

| Message | buffa | buffa (view) |
|---------|------:|------:|
| ApiResponse | 634 | 1,233 (+95%) |
| LogRecord | 319 | 2,458 (+671%) |
| AnalyticsEvent | 267 | 811 (+204%) |
| GoogleMessage1 | 674 | 896 (+33%) |
| MediaFrame | 14,503 | 34,639 (+139%) |

### JSON encode

| Message | buffa | prost | Go |
|---------|------:|------:|------:|
| ApiResponse | 534 | 589 (+10%) | 70 (−87%) |
| LogRecord | 683 | 882 (+29%) | 85 (−88%) |
| AnalyticsEvent | 505 | 533 (+6%) | 33 (−94%) |
| GoogleMessage1 | 560 | 674 (+20%) | 73 (−87%) |
| MediaFrame | 694 | 937 (+35%) | 235 (−66%) |

### JSON decode

| Message | buffa | prost | Go |
|---------|------:|------:|------:|
| ApiResponse | 484 | 205 (−58%) | 40 (−92%) |
| LogRecord | 524 | 424 (−19%) | 63 (−88%) |
| AnalyticsEvent | 168 | 152 (−10%) | 25 (−85%) |
| GoogleMessage1 | 403 | 171 (−58%) | 41 (−90%) |
| MediaFrame | 1,223 | 1,218 (−0%) | 215 (−82%) |

### Reflection decode

| Message | generated | reflect | view |
|---------|------:|------:|------:|
| ApiResponse | 600 | 248 (−59%) | 913 (+52%) |
| LogRecord | 581 | 322 (−44%) | 1,314 (+126%) |
| AnalyticsEvent | 140 | 55 (−61%) | 225 (+60%) |
| GoogleMessage1 | 744 | 193 (−74%) | 743 (−0%) |

### Reflection encode

| Message | generated | reflect |
|---------|------:|------:|
| ApiResponse | 1,943 | 472 (−76%) |
| LogRecord | 3,035 | 875 (−71%) |
| AnalyticsEvent | 406 | 70 (−83%) |
| GoogleMessage1 | 2,109 | 221 (−90%) |

### Reflection read (decode + scan all fields)

| Message | vtable | bridge | dynamic |
|---------|------:|------:|------:|
| ApiResponse | 772 (+592%) | 111 | 184 (+65%) |
| LogRecord | 1,218 (+788%) | 137 | 280 (+104%) |
| AnalyticsEvent | 222 (+562%) | 34 | 55 (+64%) |
| GoogleMessage1 | 470 (+297%) | 118 | 140 (+19%) |
