### Binary decode

| Message | buffa | buffa (view) | buffa (lazy) | prost | prost (bytes) | protobuf-v4 | Go |
|---------|------:|------:|------:|------:|------:|------:|------:|
| ApiResponse | 810 | 1,440 (+78%) | 1,361 (+68%) | 784 (−3%) | 671 (−17%) | 681 (−16%) | 268 (−67%) |
| LogRecord | 757 | 2,056 (+172%) | 2,331 (+208%) | 686 (−9%) | 643 (−15%) | 796 (+5%) | 248 (−67%) |
| AnalyticsEvent | 195 | 318 (+63%) | 18,307 (+9294%) | 251 (+29%) | 198 (+2%) | 338 (+73%) | 91 (−53%) |
| GoogleMessage1 | 1,004 | 1,249 (+24%) | 1,860 (+85%) | 971 (−3%) | 910 (−9%) | 641 (−36%) | 339 (−66%) |
| MediaFrame | 16,793 | 70,502 (+320%) | 66,856 (+298%) | 9,000 (−46%) | 21,511 (+28%) | 17,480 (+4%) | 1,264 (−92%) |

### Binary encode

| Message | buffa | buffa (view) | buffa (lazy) | prost | prost (bytes) | protobuf-v4 | Go |
|---------|------:|------:|------:|------:|------:|------:|------:|
| ApiResponse | 2,616 | 2,490 (−5%) | 2,612 (−0%) | 1,788 (−32%) | — | 1,039 (−60%) | 559 (−79%) |
| LogRecord | 4,226 | 4,678 (+11%) | 5,147 (+22%) | 3,251 (−23%) | — | 1,634 (−61%) | 304 (−93%) |
| AnalyticsEvent | 594 | 606 (+2%) | 23,840 (+3912%) | 356 (−40%) | — | 517 (−13%) | 159 (−73%) |
| GoogleMessage1 | 2,552 | 2,490 (−2%) | 3,330 (+30%) | 1,800 (−29%) | — | 898 (−65%) | 360 (−86%) |
| MediaFrame | 42,959 | 46,369 (+8%) | 47,690 (+11%) | 36,852 (−14%) | — | 10,522 (−76%) | 1,681 (−96%) |

### Build + binary encode

| Message | buffa | buffa (view) |
|---------|------:|------:|
| ApiResponse | 754 | 1,612 (+114%) |
| LogRecord | 494 | 2,865 (+480%) |
| AnalyticsEvent | 524 | 1,106 (+111%) |
| GoogleMessage1 | 899 | 1,187 (+32%) |
| MediaFrame | 21,746 | 57,325 (+164%) |

### JSON encode

| Message | buffa | prost | Go |
|---------|------:|------:|------:|
| ApiResponse | 781 | 789 (+1%) | 117 (−85%) |
| LogRecord | 1,023 | 1,130 (+11%) | 139 (−86%) |
| AnalyticsEvent | 713 | 788 (+10%) | 51 (−93%) |
| GoogleMessage1 | 822 | 839 (+2%) | 126 (−85%) |
| MediaFrame | 1,082 | 1,065 (−2%) | 213 (−80%) |

### JSON decode

| Message | buffa | prost | Go |
|---------|------:|------:|------:|
| ApiResponse | 663 | 289 (−56%) | 71 (−89%) |
| LogRecord | 770 | 677 (−12%) | 111 (−86%) |
| AnalyticsEvent | 263 | 234 (−11%) | 46 (−83%) |
| GoogleMessage1 | 640 | 251 (−61%) | 72 (−89%) |
| MediaFrame | 1,937 | 1,907 (−2%) | 272 (−86%) |

### Reflection decode

| Message | generated | reflect | view |
|---------|------:|------:|------:|
| ApiResponse | 790 | 304 (−62%) | 1,350 (+71%) |
| LogRecord | 746 | 467 (−37%) | 1,824 (+145%) |
| AnalyticsEvent | 204 | 84 (−59%) | 292 (+43%) |
| GoogleMessage1 | 974 | 210 (−78%) | 1,167 (+20%) |

### Reflection encode

| Message | generated | reflect |
|---------|------:|------:|
| ApiResponse | 2,307 | 741 (−68%) |
| LogRecord | 4,066 | 1,268 (−69%) |
| AnalyticsEvent | 558 | 109 (−80%) |
| GoogleMessage1 | 2,516 | 372 (−85%) |

### Reflection read (decode + scan all fields)

| Message | vtable | bridge | dynamic |
|---------|------:|------:|------:|
| ApiResponse | 894 (+475%) | 155 | 229 (+47%) |
| LogRecord | 1,536 (+719%) | 188 | 359 (+91%) |
| AnalyticsEvent | 290 (+466%) | 51 | 85 (+65%) |
| GoogleMessage1 | 656 (+375%) | 138 | 156 (+13%) |
