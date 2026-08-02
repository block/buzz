import i18n from "i18next";
import { initReactI18next } from "react-i18next";

const resources = {
  "en-US": {
    translation: {
      admin: "Admin",
      feedback: "Feedback",
      language: "Language",
      reports: "Reports",
      loading: "Loading…",
      retry: "Retry",
      accessDenied: "Access denied",
      actedOn: "Acted on",
      allCommunities: "All communities",
      anyStatus: "Any status",
      anyTime: "Any time",
      attachments: "Attachments",
      author: "Author",
      backToFeedback: "Back to feedback",
      backToReports: "Back to reports",
      bug: "Bug",
      community: "Community",
      created: "Created",
      deleted: "Deleted",
      event: "Event",
      feedbackDescription: "Recent product feedback from across Buzz.",
      feedbackDetail: "Feedback detail",
      feedbackDetailDescription:
        "The complete feedback submission and its source.",
      lastDay: "Last 24 hours",
      lastMonth: "Last 30 days",
      lastWeek: "Last 7 days",
      loadFailed: "Could not load data",
      message: "Message",
      messageUnavailable:
        "Message content is unavailable. It may have expired or been removed from event storage.",
      moderation: "Moderation",
      needsAction: "Needs action",
      needsWork: "Needs work",
      noMatchingFeedback: "No matching feedback.",
      noNote: "No note provided.",
      noRecords: "No records.",
      note: "Note",
      openReports: "Open reports",
      openReportsDescription: "Review reports across every Buzz community.",
      praise: "Praise",
      product: "Product",
      received: "Received",
      reporter: "Reporter",
      reportDetail: "Report detail",
      reportDetailDescription: "The full report as submitted to the relay.",
      searchFeedback: "Search feedback",
      status: "Status",
      submissionCount: "{{filtered}} of {{total}} submissions",
      submitted: "Submitted",
      submittedBy: "Submitted by",
      target: "Target",
      uncategorized: "Uncategorized",
    },
  },
  "pt-BR": {
    translation: {
      admin: "Administração",
      feedback: "Feedback",
      language: "Idioma",
      reports: "Denúncias",
      loading: "Carregando…",
      retry: "Tentar novamente",
      accessDenied: "Acesso negado",
      actedOn: "Tratado",
      allCommunities: "Todas as comunidades",
      anyStatus: "Qualquer status",
      anyTime: "Qualquer período",
      attachments: "Anexos",
      author: "Autor",
      backToFeedback: "Voltar ao feedback",
      backToReports: "Voltar às denúncias",
      bug: "Erro",
      community: "Comunidade",
      created: "Criado",
      deleted: "Excluído",
      event: "Evento",
      feedbackDescription: "Feedback recente sobre o produto em todo o Buzz.",
      feedbackDetail: "Detalhes do feedback",
      feedbackDetailDescription: "O envio completo do feedback e sua origem.",
      lastDay: "Últimas 24 horas",
      lastMonth: "Últimos 30 dias",
      lastWeek: "Últimos 7 dias",
      loadFailed: "Não foi possível carregar os dados",
      message: "Mensagem",
      messageUnavailable:
        "O conteúdo da mensagem não está disponível. Ele pode ter expirado ou sido removido do armazenamento de eventos.",
      moderation: "Moderação",
      needsAction: "Requer ação",
      needsWork: "Precisa melhorar",
      noMatchingFeedback: "Nenhum feedback encontrado.",
      noNote: "Nenhuma observação informada.",
      noRecords: "Nenhum registro.",
      note: "Observação",
      openReports: "Denúncias abertas",
      openReportsDescription:
        "Revise denúncias de todas as comunidades do Buzz.",
      praise: "Elogio",
      product: "Produto",
      received: "Recebido",
      reporter: "Denunciante",
      reportDetail: "Detalhes da denúncia",
      reportDetailDescription: "A denúncia completa conforme enviada ao relay.",
      searchFeedback: "Buscar feedback",
      status: "Status",
      submissionCount: "{{filtered}} de {{total}} envios",
      submitted: "Enviado",
      submittedBy: "Enviado por",
      target: "Alvo",
      uncategorized: "Sem categoria",
    },
  },
} as const;

export type SupportedLanguage = "pt-BR" | "en-US";
const storageKey = "buzz.locale";
const persisted = window.localStorage.getItem(storageKey);
const initialLanguage =
  persisted === "pt-BR" || persisted === "en-US"
    ? persisted
    : navigator.language.toLowerCase().startsWith("pt")
      ? "pt-BR"
      : "en-US";

void i18n.use(initReactI18next).init({
  fallbackLng: "en-US",
  interpolation: { escapeValue: false },
  lng: initialLanguage,
  resources,
});
document.documentElement.lang = initialLanguage;

export async function setLanguage(language: SupportedLanguage): Promise<void> {
  window.localStorage.setItem(storageKey, language);
  document.documentElement.lang = language;
  await i18n.changeLanguage(language);
}

export function currentLanguage(): SupportedLanguage {
  return (i18n.resolvedLanguage ?? i18n.language).toLowerCase().startsWith("pt")
    ? "pt-BR"
    : "en-US";
}

export { i18n };
