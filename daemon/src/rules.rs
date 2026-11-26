pub const CORE: &[&str] = &[
    "assets1.xboxlive.com",
    "assets2.xboxlive.com",
    "xvcf1.xboxlive.com",
    "xvcf2.xboxlive.com",
    "d1.xboxlive.com",
    "d2.xboxlive.com",
    "5.xb.delivery.microsoft.com",
    "assets1.xboxlive.com.edgesuite.net",
    "cl-glcb907925.gcdn.co",
    "fg.microsoft.map.fastly.net",
];
pub const LEGACY: &[&str] = &["dlassets.xboxlive.com", "dlassets2.xboxlive.com"];
pub const SRV: &[&str] = &[
    "assets-srvs.xboxlive.com",
    "assets-srvs-net.trafficmanager.net",
    "assets-srvs-apac.delivery.microsoft.com",
    "assets-srvs-na.delivery.microsoft.com",
];
pub const FILES: &[&str] = &[
    "IP.XboxCn1.txt",
    "IP.XboxCn2.txt",
    "IP.XboxApp.txt",
    "IP.Akamai.txt",
    "IP.AkamaiV2.txt",
];

pub struct Rule {
    pub id: &'static str,
    pub hosts: &'static [&'static str],
    pub files: &'static [&'static str],
    pub urls: &'static [&'static str],
}
pub const GROUPS: &[Rule] = &[
    Rule {
        id: "xbox_cn1",
        hosts: &[
            "assets1.xboxlive.cn",
            "assets2.xboxlive.cn",
            "d1.xboxlive.cn",
            "d2.xboxlive.cn",
        ],
        files: &["IP.XboxCn1.txt"],
        urls: &[
            "http://assets1.xboxlive.cn/Z/routing/extraextralarge.txt",
            "http://assets1.xboxlive.cn/8/71359f90-2c86-4118-8df8-8546ebd47956/0698b936-d300-4451-b9a0-0be0514bbbe5/1.4206.46191.0.88d41e86-b22b-49f0-bb08-9879d39f287d/Microsoft.254428597CFE2_1.4206.46191.0_neutral__8wekyb3d8bbwe_xs.xvc",
            "http://assets1.xboxlive.cn/1/13bab257-5971-49d3-b885-d5238a99668e/3d263e92-93cd-4f9b-90c7-5438150cecbf/3.688.109.0.14726359-c4ce-4a75-b063-d5144b3e9868/Microsoft.624F8B84B80_3.688.109.0_x64__8wekyb3d8bbwe.msixvc",
        ],
    },
    Rule {
        id: "xbox_cn2",
        hosts: &["dlassets.xboxlive.cn", "dlassets2.xboxlive.cn"],
        files: &["IP.XboxCn2.txt"],
        urls: &[
            "http://dlassets.xboxlive.cn/public/content/0044aef3-bd7e-4478-8782-6191546dcd9a/f6585e19-765c-49c3-ba77-326c3766e05a/1.0.4.0.60281fce-b9c0-49f6-a2b6-027abad22637/ACBF_1.0.4.0_x64__b6krnev7r9sf8",
        ],
    },
    Rule {
        id: "xbox_app",
        hosts: &[
            "dl.delivery.mp.microsoft.com",
            "tlu.dl.delivery.mp.microsoft.com",
            "1d.tlu.dl.delivery.mp.microsoft.com",
        ],
        files: &["IP.XboxApp.txt"],
        urls: &[],
    },
    Rule {
        id: "akamai",
        hosts: &[],
        files: &["IP.AkamaiV2.txt", "IP.Akamai.txt"],
        urls: &["http://xvcf1.xboxlive.com/Z/routing/extraextralarge.txt"],
    },
];
pub fn group(id: &str) -> anyhow::Result<&'static Rule> {
    GROUPS
        .iter()
        .find(|r| r.id == id)
        .ok_or_else(|| anyhow::anyhow!("unknown group: {id}"))
}
pub fn target(host: &str) -> Option<&'static str> {
    if CORE.contains(&host) {
        Some("assets1.xboxlive.cn")
    } else if LEGACY.contains(&host) {
        Some("dlassets.xboxlive.cn")
    } else {
        None
    }
}
