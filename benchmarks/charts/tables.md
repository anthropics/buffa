### Binary decode

| Message | buffa | buffa (view) | prost | protobuf-v4 | Go |
|---------|------:|------:|------:|------:|------:|
| ApiResponse | 834 | 1,413 (+69%) | 766 (−8%) | 712 (−15%) | 270 (−68%) |
| LogRecord | 768 | 1,921 (+150%) | 681 (−11%) | 873 (+14%) | 249 (−68%) |
| AnalyticsEvent | 198 | 316 (+60%) | 252 (+28%) | 358 (+81%) | 91 (−54%) |
| GoogleMessage1 | 1,024 | 1,322 (+29%) | 998 (−3%) | 648 (−37%) | 344 (−66%) |

### Binary encode

| Message | buffa | prost | protobuf-v4 | Go |
|---------|------:|------:|------:|------:|
| ApiResponse | 2,613 | 1,680 (−36%) | 1,049 (−60%) | 556 (−79%) |
| LogRecord | 4,102 | 3,000 (−27%) | 1,666 (−59%) | 302 (−93%) |
| AnalyticsEvent | 656 | 366 (−44%) | 511 (−22%) | 159 (−76%) |
| GoogleMessage1 | 2,644 | 1,867 (−29%) | 872 (−67%) | 358 (−86%) |

### JSON encode

| Message | buffa | prost | Go |
|---------|------:|------:|------:|
| ApiResponse | 867 | 805 (−7%) | 116 (−87%) |
| LogRecord | 1,312 | 1,083 (−17%) | 140 (−89%) |
| AnalyticsEvent | 777 | 758 (−2%) | 51 (−93%) |
| GoogleMessage1 | 1,021 | 830 (−19%) | 128 (−88%) |

### JSON decode

| Message | buffa | prost | Go |
|---------|------:|------:|------:|
| ApiResponse | 718 | 293 (−59%) | 70 (−90%) |
| LogRecord | 797 | 690 (−13%) | 110 (−86%) |
| AnalyticsEvent | 265 | 235 (−11%) | 46 (−83%) |
| GoogleMessage1 | 646 | 255 (−60%) | 73 (−89%) |
