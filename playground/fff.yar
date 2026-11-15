# [정답셋] PC 내 외부 유출 금지 자격 증명 및 인증키 예제 리스트

본 데이터는 보안 솔루션, 정적 코드 분석기(SAST), DLP(데이터 유출 방지) 검출기의 성능 테스트 및 AI 학습을 위한 정답(Ground Truth) 데이터셋입니다. 

---

## 1. Cloud Infrastructure Credentials (클라우드 인프라 자격 증명)

### AWS Access Key ID
*   **길이/특징:** 20자 알팬뉴메릭, 대문자 기반, `AKIA` 또는 `ASIA` 접두사로 시작
```text
AKIAIOSFODNN7EXAMPLE
```

### AWS Secret Access Key
*   **길이/특징:** 40자 대소문자 및 일부 특수문자(`/`, `+`) 조합
```text
wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
```

### Google Cloud Platform (GCP) Service Account Key
*   **길이/특징:** JSON 포맷 내부의 40자 `private_key_id` 및 PKCS#8 형식의 `private_key` 블록
```json
{
  "type": "service_account",
  "project_id": "production-data-project-2026",
  "private_key_id": "abcd1234efgh5678ijkl9012mnop34567890efgh",
  "private_key": "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQC3...\n-----END PRIVATE KEY-----\n",
  "client_email": "sa-scanner@://gserviceaccount.com"
}
```

---

## 2. Dev Tools & Collaboration Tokens (개발 및 협업 툴 토큰)

### GitHub Personal Access Token (PAT)
*   **길이/특징:** 과거 40자 무작위 문자열 또는 현재 접두사(`ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_`)가 포함된 40자 형태
```text
ghp_vY72Xm89vM39KmN21pLz62XqQ98zW12aBcDe
```

### GitLab Personal Access Token
*   **길이/특징:** `glpat-` 접두사로 시작하는 26자 이상의 문자열
```text
glpat-fX28szm91LkWqPoxZ71A
```

### Slack Bot Token
*   **길이/특징:** `xoxb-` 접두사로 시작하며 대시(`-`)로 구분된 세 문자열 구조
```text
xoxb-123456789012-1234567890123-abcdefghijklmnopqrstuvwx
```

---

## 3. Web & AI API Keys (인증 및 AI 서비스 키)

### JSON Web Token (JWT) - Bearer Token
*   **길이/특징:** Base64URL로 인코딩된 세 가지 세그먼트가 마침표(`.`)로 연결됨. 항상 `eyJ`로 시작
```text
eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c
```

### OpenAI API Key
*   **길이/특징:** `sk-` 또는 최신 포맷인 `sk-proj-` 접두사로 시작하는 긴 무작위 난수 구조
```text
sk-proj-AbCdEfGhIjKlMnOpQrStUvWxYzAbCdEfGhIjKlMnOpQrStUvWxYz
```

---

## 4. Private Cryptographic Keys (암호화 개인키 파일 내용)

### RSA Private Key
*   **길이/특징:** PEM 형식의 고유한 헤더와 푸터 블록 내부의 고밀도 Base64 텍스트
```text
-----BEGIN RSA PRIVATE KEY-----
MIIEogIBAAKCAQEAn9sVzuX7k8q23AbCdEfGhIjKlMnOpQrStUvWxYz12345
67890abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ+/=
-----END RSA PRIVATE KEY-----
```

### Unencrypted Private Key (PKCS#8)
*   **길이/특징:** 알고리즘 범용 암호화 개인키의 표준 헤더 구조
```text
-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDAbCdEfGhI
jKlMnOpQrStUvWxYz1234567890abcdefghijklmnopqrstuvwxyz+++++/=
-----END PRIVATE KEY-----
```

---

## 5. Database Connection Strings (데이터베이스 접속 정보)

### MongoDB Connection URI
*   **길이/특징:** 프로토콜 명시 후 `ID:Password@Host/DBName` 구조를 띰 (패스워드 특수문자는 % 인코딩)
```text
mongodb+srv://db_admin_root:p%40ssw0rd123@cluster0.abcde.mongodb.net/production_db?retryWrites=true&w=majority
```

### MySQL / PostgreSQL Connection URI
*   **길이/특징:** 포트 번호(`3306`, `5432`)와 자격 증명이 날것으로 노출되는 형태
```text
mysql://sys_user:SecureP%40ss55@://amazonaws.com
```
```text
postgresql://postgres:rootPass99!@localhost:5432/user_management?sslmode=verify-full
```
```markdown
<FollowUp>
이 예제 파일들을 기반으로 **AI 모델(예: LLM 미세조정, 정규식 자동 생성기)을 학습시키거나 정답 매칭 알고리즘을 테스트**하려는 단계이신가요? 혹시 수집 결과에서 정상 코드와 오탐(False Positive)을 구분하기 위한 **"안전한 문자열(가짜 샘플)" 예제**도 추가로 필요하신지 말씀해 주세요.
</FollowUp>
