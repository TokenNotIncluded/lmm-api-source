// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
// Language order: en, zh-CN, zh-TW, fr, ja, ru, vi.
pub(super) type Labels = [&'static str; 7];

pub(super) const SIGN_IN_TITLE: Labels = [
    "Sign in to continue",
    "登录以继续",
    "登入以繼續",
    "Connectez-vous pour continuer",
    "ログインして続行",
    "Войдите, чтобы продолжить",
    "Đăng nhập để tiếp tục",
];

pub(super) const CONSENT_TITLE: Labels = [
    "Review app access",
    "核对应用请求的权限",
    "確認應用程式要求的權限",
    "Vérifiez les accès demandés",
    "アプリが求める権限を確認",
    "Проверьте разрешения приложения",
    "Xem lại quyền ứng dụng yêu cầu",
];

pub(super) const APPLICATION: Labels = [
    "Application",
    "应用",
    "應用程式",
    "Application",
    "アプリ",
    "Приложение",
    "Ứng dụng",
];

pub(super) const RESOURCE: Labels = [
    "Resource",
    "目标资源",
    "目標資源",
    "Ressource",
    "対象リソース",
    "Ресурс",
    "Tài nguyên",
];

pub(super) const ACCOUNT: Labels = [
    "Account",
    "账户",
    "帳戶",
    "Compte",
    "アカウント",
    "Аккаунт",
    "Tài khoản",
];

pub(super) const INSTALLATION: Labels = [
    "Installation",
    "安装实例",
    "安裝實例",
    "Installation",
    "インストール先 ID",
    "Экземпляр приложения",
    "Bản cài đặt",
];

pub(super) const DEVICE: Labels = [
    "Device",
    "设备",
    "裝置",
    "Appareil",
    "デバイス",
    "Устройство",
    "Thiết bị",
];

pub(super) const PERMISSIONS: Labels = [
    "Permissions",
    "请求的权限",
    "要求的權限",
    "Autorisations demandées",
    "要求された権限",
    "Запрашиваемые разрешения",
    "Quyền được yêu cầu",
];

pub(super) const GROUPS: Labels = [
    "Approved groups",
    "获准使用的分组",
    "允許使用的群組",
    "Groupes autorisés",
    "許可されたグループ",
    "Разрешённые группы",
    "Nhóm được phép sử dụng",
];

pub(super) const READ: Labels = [
    "Read the model catalog and your personal wallet balance.",
    "查看模型目录和你的个人钱包余额。",
    "查看模型目錄和你的個人錢包餘額。",
    "Consulter le catalogue de modèles et le solde de votre portefeuille personnel.",
    "モデル一覧と個人ウォレットの残高を確認する。",
    "Просматривать каталог моделей и баланс вашего личного кошелька.",
    "Xem danh mục mô hình và số dư ví cá nhân của bạn.",
];

pub(super) const OPENID: Labels = [
    "Share your stable account identifier with this application.",
    "向此应用提供你的固定账户标识符。",
    "向此應用程式提供你的固定帳戶識別碼。",
    "Communiquer à cette application l’identifiant stable de votre compte.",
    "アカウントを識別する固定 ID をこのアプリに共有する。",
    "Передать этому приложению постоянный идентификатор вашего аккаунта.",
    "Chia sẻ mã định danh cố định của tài khoản với ứng dụng này.",
];

pub(super) const INVOKE: Labels = [
    "Call models in the approved groups and charge your personal account within your budget.",
    "调用获准分组中的模型，费用从你的个人账户扣除，上限为你设置的预算。",
    "呼叫獲准群組中的模型，費用從你的個人帳戶扣除，上限為你設定的預算。",
    "Appeler les modèles des groupes autorisés et débiter votre compte personnel dans la limite de votre budget.",
    "許可されたグループのモデルを呼び出し、設定した予算の範囲で個人アカウントに課金する。",
    "Вызывать модели из разрешённых групп и списывать средства с вашего личного счёта в пределах бюджета.",
    "Gọi mô hình trong các nhóm được phép và trừ phí từ tài khoản cá nhân trong phạm vi ngân sách bạn đặt.",
];

pub(super) const SIGN_IN_HELP: Labels = [
    "Sign in to LMM in a new tab, then return here to review the request.",
    "在新标签页登录 LMM，然后回到此页核对授权请求。",
    "在新分頁登入 LMM，再回到此頁確認授權要求。",
    "Connectez-vous à LMM dans un nouvel onglet, puis revenez ici pour vérifier la demande.",
    "新しいタブで LMM にログインし、このページに戻ってリクエストを確認してください。",
    "Войдите в LMM в новой вкладке, затем вернитесь сюда, чтобы проверить запрос.",
    "Đăng nhập vào LMM trong tab mới, sau đó quay lại đây để xem lại yêu cầu.",
];

pub(super) const OPEN_SIGN_IN: Labels = [
    "Open sign in",
    "前往登录",
    "前往登入",
    "Ouvrir la page de connexion",
    "ログイン画面を開く",
    "Открыть страницу входа",
    "Mở trang đăng nhập",
];

pub(super) const CONTINUE: Labels = [
    "Continue after sign in",
    "登录后继续",
    "登入後繼續",
    "Continuer après la connexion",
    "ログイン後に続行",
    "Продолжить после входа",
    "Tiếp tục sau khi đăng nhập",
];

pub(super) const BUDGET: Labels = [
    "Lifetime budget in credits",
    "整个授权期间的总预算（积分）",
    "整個授權期間的總預算（點數）",
    "Budget total pour la durée de l’autorisation (en crédits)",
    "許可期間全体の予算（クレジット）",
    "Бюджет на весь срок доступа (в кредитах)",
    "Tổng ngân sách trong thời hạn cấp quyền (điểm tín dụng)",
];

pub(super) const BUDGET_HELP: Labels = [
    "500,000 credits = 1 USD. Pending holds count toward this limit.",
    "500,000 积分等于 1 美元。已预留、尚未结算的积分也计入预算。",
    "500,000 點等於 1 美元。已保留、尚未結算的點數也計入預算。",
    "500 000 crédits valent 1 dollar américain. Les crédits réservés en attente de règlement comptent dans ce plafond.",
    "500,000 クレジットは 1 米ドルです。精算前に確保されたクレジットも予算に含まれます。",
    "500 000 кредитов равны 1 доллару США. Зарезервированные суммы тоже учитываются в этом лимите.",
    "500.000 điểm tín dụng tương đương 1 USD. Số điểm đang tạm giữ cũng được tính vào hạn mức này.",
];

pub(super) const ALLOW_INVOKE: Labels = [
    "Allow scoped model access",
    "允许按上述范围调用模型",
    "允許在上述範圍內呼叫模型",
    "Autoriser les appels de modèles dans ce périmètre",
    "この範囲でモデルの呼び出しを許可",
    "Разрешить вызовы моделей в указанных пределах",
    "Cho phép gọi mô hình trong phạm vi trên",
];

pub(super) const ALLOW_READ: Labels = [
    "Allow read-only access",
    "允许只读访问",
    "允許唯讀存取",
    "Autoriser la lecture seule",
    "読み取りのみ許可",
    "Разрешить только чтение",
    "Chỉ cho phép đọc",
];

pub(super) const DENY: Labels = [
    "Deny",
    "拒绝",
    "拒絕",
    "Refuser",
    "拒否",
    "Отклонить",
    "Từ chối",
];

pub(super) const REVOKE: Labels = [
    "You can revoke this access from your account at any time.",
    "你随时可以在账户中撤销此授权。",
    "你隨時可以在帳戶中撤銷這項授權。",
    "Vous pouvez révoquer cet accès à tout moment depuis votre compte.",
    "この許可は、アカウントからいつでも取り消せます。",
    "Вы можете в любой момент отозвать этот доступ в своём аккаунте.",
    "Bạn có thể thu hồi quyền truy cập này bất cứ lúc nào trong tài khoản của mình.",
];

pub(super) const LANGUAGE: Labels = [
    "Language",
    "语言",
    "語言",
    "Langue",
    "言語",
    "Язык",
    "Ngôn ngữ",
];

pub(super) const TITLE: Labels = [
    "LMM authorization",
    "LMM 应用授权",
    "LMM 應用程式授權",
    "Autorisation LMM",
    "LMM アプリ認可",
    "Авторизация LMM",
    "Cấp quyền LMM",
];
