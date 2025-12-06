use {
	anyhow::anyhow,
	jsonwebtoken::{Algorithm, DecodingKey, Validation, decode},
	serde::{Deserialize, Serialize},
};
/*
生产环境和测试环境的privy token不一样
生产:
eyJhbGciOiJFUzI1NiIsInR5cCI6IkpXVCIsImtpZCI6IlVFZ1loY0dSNWt0cVRYSmdDSmNpYTVrc29RbTdhVC10NTBRMTRjSXZxZmcifQ.eyJzaWQiOiJjbWh3cnBocGcwMGt2ankwZHhsZ3c1YW1lIiwiaXNzIjoicHJpdnkuaW8iLCJpYXQiOjE3NjI5OTgzNjYsImF1ZCI6ImNtOG55ODRuazAyMXoxbmdya2U4bmNubGciLCJzdWIiOiJkaWQ6cHJpdnk6Y21odmF3OHVoMDA3M2llMGNqeG1wZWY3ciIsImV4cCI6MTc2MzAwMTk2Nn0.ykCVFnJYXfYKjQOv4HOQXIVADMY7VzJq9pSGS6fOzARHMmxa7INwmE-rACgN0uf4NMQuHcYrITAmLBctjAE4wg
{
	"alg": "ES256",
	"typ": "JWT",
	"kid": "UEgYhcGR5ktqTXJgCJcia5ksoQm7aT-t50Q14cIvqfg"
}

{
	"sid": "cmhwrphpg00kvjy0dxlgw5ame",
	"iss": "privy.io",
	"iat": 1762998366,
	"aud": "cm8ny84nk021z1ngrke8ncnlg",
	"sub": "did:privy:cmhvaw8uh0073ie0cjxmpef7r",
	"exp": 1763001966
}

测试
eyJhbGciOiJFUzI1NiIsInR5cCI6IkpXVCIsImtpZCI6IndSUFVRemY4M3NQX2tZVHNHWGt0eUtFaDBVOU1OWmZ3VzlqVWE2VTVlSFkifQ.eyJhaWQiOiJjbTd2bWl5ZnYwMGl0Z2h1amcwNzZ2am1pIiwiYXR0IjoicGF0Iiwic2lkIjoiY21odmtjenV5MDA5dGt5MGNvYzFjenpsaSIsImlzcyI6InByaXZ5LmlvIiwiaWF0IjoxNzYyOTk0ODA2LCJhdWQiOiJwcml2eS5jbGlja2Jhci5jbyIsInN1YiI6ImRpZDpwcml2eTpjbWMxcjhsbWswMTRtbGQwbnduMHB3ZXdxIiwiZXhwIjoxNzYyOTk4NDA2fQ.agwcN6wsOD1XqSTMECk2N00245UjwR7rBrJKE1n6k3lfekFHv2pMD2E0TEAmmjhrl8-fXUdN84FD-3Jak-fADA
{
	"alg": "ES256",
	"typ": "JWT",
	"kid": "wRPUQzf83sP_kYTsGXktyKEh0U9MNZfwW9jUa6U5eHY"
}

{
	"aid": "cm7vmiyfv00itghujg076vjmi",
	"att": "pat",
	"sid": "cmhvkczuy009tky0coc1czzli",
	"iss": "privy.io",
	"iat": 1762994806,
	"aud": "privy.clickbar.co",
	"sub": "did:privy:cmc1r8lmk014mld0nwn0pwewq",
	"exp": 1762998406
}

	可以看到生产的aud是appid 而测试的aud是privy.clickbar.co

但是测试环境也有过aud会是appid的情况
eyJhbGciOiJFUzI1NiIsInR5cCI6IkpXVCIsImtpZCI6IndSUFVRemY4M3NQX2tZVHNHWGt0eUtFaDBVOU1OWmZ3VzlqVWE2VTVlSFkifQ.eyJzaWQiOiJjbWh2a2N6dXkwMDl0a3kwY29jMWN6emxpIiwiaXNzIjoicHJpdnkuaW8iLCJpYXQiOjE3NjI5OTg0NTgsImF1ZCI6ImNtN3ZtaXlmdjAwaXRnaHVqZzA3NnZqbWkiLCJzdWIiOiJkaWQ6cHJpdnk6Y21jMXI4bG1rMDE0bWxkMG53bjBwd2V3cSIsImV4cCI6MTc2MzAwMjA1OH0.Y4tyH9HEtDdT0fcaaVhTdfO2yspbpVlF4Y0E4gI0MVrNQ7fdLuazGxNb-jqsAHMFi5AuAqv9w7zwYJoJxtkdKw

{
	"alg": "ES256",
	"typ": "JWT",
	"kid": "wRPUQzf83sP_kYTsGXktyKEh0U9MNZfwW9jUa6U5eHY"
}


{
	"sid": "cmhvkczuy009tky0coc1czzli",
	"iss": "privy.io",
	"iat": 1762998458,
	"aud": "cm7vmiyfv00itghujg076vjmi",
	"sub": "did:privy:cmc1r8lmk014mld0nwn0pwewq",
	"exp": 1763002058
}
*/

/// Privy JWT声明结构
#[derive(Clone, Debug, Serialize, Deserialize)]
struct PrivyClaims {
	pub aid: Option<String>, //测试有生产没有
	pub aud: String,
	pub exp: u64,    // Expiration timestamp
	pub iss: String, // Issuer
	pub sub: String, // User ID (Privy DID)
}

impl PrivyClaims {
	/// 验证Privy JWT过期
	pub fn valid(&self, app_id: &str) -> anyhow::Result<()> {
		if self.aid.is_none() && self.aud != app_id {
			return Err(anyhow!("aud claim must be your Privy App ID."));
		} else if let Some(aid) = &self.aid
			&& aid != app_id
		{
			return Err(anyhow!("aid claim must be your Privy App ID."));
		}
		if self.iss != "privy.io" {
			return Err(anyhow!("iss claim must be 'privy.io'"));
		}
		Ok(())
	}
}

/// 验证并解析Privy JWT令牌
///
/// # 参数
///
/// * `access_token` - JWT令牌字符串
/// * `pem_key` - 用于验证签名的PEM格式公钥
/// * `app_id` - 应用ID，用于验证令牌的受众
///
/// # 返回
///
/// 成功则返回PrivyClaims，否则返回错误
pub fn check_privy_jwt(token: &str, pem_key: &str, app_id: &str) -> anyhow::Result<String> {
	// 去掉 "Bearer " 前缀（不区分大小写）
	let token = token.trim_start();
	let token = token.strip_prefix("Bearer ").or_else(|| token.strip_prefix("bearer ")).or_else(|| token.strip_prefix("BEARER ")).unwrap_or(token);

	// 设置验证参数
	let mut validation = Validation::new(Algorithm::ES256);
	validation.set_audience(&["privy.clickbar.co", app_id]);

	// 解析JWT
	// 处理 PEM key：先尝试直接解析，如果失败则尝试将字面字符串 \n 替换为真正的换行符
	// 因为环境变量中的 \n 可能被当作字面字符串而不是换行符
	let key = match DecodingKey::from_ec_pem(pem_key.as_bytes()) {
		Ok(key) => key,
		Err(_) => {
			// 如果直接解析失败，尝试将 \n 字符串替换为真正的换行符
			let pem_key_normalized = pem_key.replace("\\n", "\n");
			DecodingKey::from_ec_pem(pem_key_normalized.as_bytes())?
		}
	};

	// 使用标准结构解析JWT 这一步就会验证是否过期
	let token_data = decode::<PrivyClaims>(token, &key, &validation)?;
	// let token_data = match decode::<PrivyClaims>(token, &key, &validation) {
	// 	Ok(token_data) => token_data,
	// 	Err(e) => {
	// 		error!("Failed to decode JWT: {}", e);
	// 		return Err(anyhow!("Failed to decode JWT: {}", e));
	// 	}
	// };

	let claims = token_data.claims;
	// 验证令牌过期
	claims.valid(app_id)?;
	// 去除user_id中的"did:privy:"前缀
	Ok(claims.sub.strip_prefix("did:privy:").unwrap_or(&claims.sub).to_string())
}
