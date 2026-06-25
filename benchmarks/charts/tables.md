### Binary decode

| Message | buffa | buffa (view) | buffa (lazy) | prost | prost (bytes) | protobuf-v4 | Go |
|---------|------:|------:|------:|------:|------:|------:|------:|
| ApiResponse | 575 | 872 (+52%) | 912 (+59%) | 550 (−4%) | 546 (−5%) | 430 (−25%) | 175 (−70%) |
| LogRecord | 572 | 1,336 (+134%) | 1,716 (+200%) | 481 (−16%) | 477 (−17%) | 555 (−3%) | 161 (−72%) |
| AnalyticsEvent | 123 | 225 (+83%) | 11,873 (+9538%) | 148 (+20%) | 129 (+5%) | 222 (+80%) | 57 (−54%) |
| GoogleMessage1 | 601 | 786 (+31%) | 1,452 (+142%) | 698 (+16%) | 669 (+11%) | 373 (−38%) | 263 (−56%) |
| MediaFrame | 10,619 | 41,441 (+290%) | 40,678 (+283%) | 6,002 (−43%) | 18,432 (+74%) | 11,005 (+4%) | 1,890 (−82%) |

### Binary encode

| Message | buffa | buffa (view) | buffa (lazy) | prost | prost (bytes) | protobuf-v4 | Go |
|---------|------:|------:|------:|------:|------:|------:|------:|
| ApiResponse | 1,972 | 1,955 (−1%) | 1,959 (−1%) | 1,964 (−0%) | — | 639 (−68%) | 384 (−81%) |
| LogRecord | 3,053 | 3,509 (+15%) | 3,587 (+17%) | 2,758 (−10%) | — | 1,067 (−65%) | 186 (−94%) |
| AnalyticsEvent | 404 | 426 (+6%) | 12,960 (+3109%) | 238 (−41%) | — | 307 (−24%) | 105 (−74%) |
| GoogleMessage1 | 2,122 | 2,123 (+0%) | 2,965 (+40%) | 1,816 (−14%) | — | 522 (−75%) | 232 (−89%) |
| MediaFrame | 25,727 | 27,325 (+6%) | 27,441 (+7%) | 25,402 (−1%) | — | 6,671 (−74%) | 2,423 (−91%) |

### Build + binary encode

| Message | buffa | buffa (view) |
|---------|------:|------:|
| ApiResponse | 639 | 1,224 (+91%) |
| LogRecord | 315 | 2,447 (+678%) |
| AnalyticsEvent | 267 | 802 (+200%) |
| GoogleMessage1 | 673 | 900 (+34%) |
| MediaFrame | 14,432 | 33,481 (+132%) |

### JSON encode

| Message | buffa | prost | Go |
|---------|------:|------:|------:|
| ApiResponse | 521 | 589 (+13%) | 70 (−87%) |
| LogRecord | 697 | 882 (+27%) | 85 (−88%) |
| AnalyticsEvent | 505 | 533 (+6%) | 33 (−94%) |
| GoogleMessage1 | 571 | 674 (+18%) | 73 (−87%) |
| MediaFrame | 702 | 937 (+33%) | 235 (−67%) |

### JSON decode

| Message | buffa | prost | Go |
|---------|------:|------:|------:|
| ApiResponse | 492 | 205 (−58%) | 40 (−92%) |
| LogRecord | 530 | 424 (−20%) | 63 (−88%) |
| AnalyticsEvent | 169 | 152 (−10%) | 25 (−85%) |
| GoogleMessage1 | 413 | 171 (−59%) | 41 (−90%) |
| MediaFrame | 1,235 | 1,218 (−1%) | 215 (−83%) |

### Reflection decode

| Message | generated | reflect | view |
|---------|------:|------:|------:|
| ApiResponse | 605 | 247 (−59%) | 914 (+51%) |
| LogRecord | 596 | 315 (−47%) | 1,364 (+129%) |
| AnalyticsEvent | 135 | 53 (−61%) | 224 (+66%) |
| GoogleMessage1 | 725 | 195 (−73%) | 746 (+3%) |

### Reflection encode

| Message | generated | reflect |
|---------|------:|------:|
| ApiResponse | 1,946 | 476 (−76%) |
| LogRecord | 3,055 | 885 (−71%) |
| AnalyticsEvent | 397 | 71 (−82%) |
| GoogleMessage1 | 2,127 | 223 (−90%) |

### Reflection read (decode + scan all fields)

| Message | vtable | bridge | dynamic |
|---------|------:|------:|------:|
| ApiResponse | 784 (+586%) | 114 | 186 (+63%) |
| LogRecord | 1,221 (+763%) | 141 | 283 (+100%) |
| AnalyticsEvent | 224 (+576%) | 33 | 54 (+64%) |
| GoogleMessage1 | 471 (+300%) | 118 | 142 (+21%) |
