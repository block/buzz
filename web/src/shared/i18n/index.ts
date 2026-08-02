import i18n from "i18next";
import { initReactI18next } from "react-i18next";

const resources = {
  "en-US": {
    translation: {
      common: { close: "Close", retry: "Try again" },
      language: {
        english: "English",
        label: "Language",
        portuguese: "Português",
      },
      repos: {
        emptyDescription:
          "Repositories pushed to this community will show up here. Open this community in the Buzz desktop app to start pushing code.",
        emptyTitle: "This community is empty",
        failed: "Failed to load repositories",
        name: "Name",
        newest: "Newest",
        noMatch: "No matching repositories",
        oldest: "Oldest",
        search: "Find a repository...",
        sort: "Sort repositories",
        title: "Repositories",
        trySearch: "Try adjusting your search term.",
      },
      invite: {
        acceptBuzz: "Accept invite in Buzz",
        age: "I am 18 years of age or older.",
        agree: "I agree to the Buzz Terms of Service and Privacy Policy.",
        agreePrefix: "I agree to the Buzz",
        and: "and",
        claimFailed: "Could not claim this invite.",
        download: "Download it now",
        exhausted:
          "This invite has reached its use limit. Ask for a new invite.",
        expired: "This invite has expired. Ask for a new invite.",
        invalid:
          "This invite is invalid. Check the link or ask for a new invite.",
        joinBrowser: "Join in browser",
        joining: "Joining…",
        macDescription: "Choose based on when your Mac was released.",
        macHelp:
          "Not sure? Open the Apple menu and choose About This Mac. ‘Chip: Apple M…’ means Newer Mac. ‘Processor: Intel’ means Older Mac.",
        newerMac: "Newer Mac",
        newerMacDescription:
          "2021 or later, or a late-2020 Mac with an Apple M1 chip",
        noApp: "Don't have the app?",
        olderMac: "Older Mac",
        olderMacDescription:
          "2019 or earlier, or a 2020 Mac with an Intel processor",
        privacy: "Privacy Policy",
        terms: "Terms of Service",
        title: "You're invited to",
        whichMac: "Which Mac do you have?",
      },
    },
  },
  "pt-BR": {
    translation: {
      common: { close: "Fechar", retry: "Tentar novamente" },
      language: {
        english: "English",
        label: "Idioma",
        portuguese: "Português",
      },
      repos: {
        emptyDescription:
          "Os repositórios enviados para esta comunidade aparecerão aqui. Abra esta comunidade no aplicativo Buzz para começar a enviar código.",
        emptyTitle: "Esta comunidade está vazia",
        failed: "Não foi possível carregar os repositórios",
        name: "Nome",
        newest: "Mais recentes",
        noMatch: "Nenhum repositório encontrado",
        oldest: "Mais antigos",
        search: "Buscar um repositório...",
        sort: "Ordenar repositórios",
        title: "Repositórios",
        trySearch: "Tente ajustar o termo de busca.",
      },
      invite: {
        acceptBuzz: "Aceitar convite no Buzz",
        age: "Confirmo que tenho 18 anos ou mais.",
        agree:
          "Concordo com os Termos de Serviço e a Política de Privacidade do Buzz.",
        agreePrefix: "Concordo com os",
        and: "e a",
        claimFailed: "Não foi possível aceitar este convite.",
        download: "Baixe agora",
        exhausted:
          "Este convite atingiu o limite de usos. Solicite um novo convite.",
        expired: "Este convite expirou. Solicite um novo convite.",
        invalid:
          "Este convite é inválido. Confira o link ou solicite um novo convite.",
        joinBrowser: "Entrar pelo navegador",
        joining: "Entrando…",
        macDescription: "Escolha conforme o ano e o processador do seu Mac.",
        macHelp:
          "Não tem certeza? Abra o menu Apple e escolha Sobre Este Mac. ‘Chip: Apple M…’ indica Mac recente; ‘Processador: Intel’ indica Mac antigo.",
        newerMac: "Mac recente",
        newerMacDescription:
          "2021 ou posterior, ou um Mac do fim de 2020 com chip Apple M1",
        noApp: "Ainda não tem o aplicativo?",
        olderMac: "Mac antigo",
        olderMacDescription:
          "2019 ou anterior, ou um Mac de 2020 com processador Intel",
        privacy: "Política de Privacidade",
        terms: "Termos de Serviço",
        title: "Você foi convidado para",
        whichMac: "Qual é o seu Mac?",
      },
    },
  },
} as const;

export type SupportedLanguage = "pt-BR" | "en-US";
export const LANGUAGE_STORAGE_KEY = "buzz.locale";

export function normalizeLanguage(language?: string | null): SupportedLanguage {
  return language?.toLowerCase().startsWith("pt") ? "pt-BR" : "en-US";
}

const stored = window.localStorage.getItem(LANGUAGE_STORAGE_KEY);
const initialLanguage =
  stored === "pt-BR" || stored === "en-US"
    ? stored
    : normalizeLanguage(navigator.language);

void i18n.use(initReactI18next).init({
  fallbackLng: "en-US",
  interpolation: { escapeValue: false },
  lng: initialLanguage,
  resources,
});
document.documentElement.lang = initialLanguage;

export async function setLanguage(language: SupportedLanguage): Promise<void> {
  window.localStorage.setItem(LANGUAGE_STORAGE_KEY, language);
  document.documentElement.lang = language;
  await i18n.changeLanguage(language);
}

export function currentLanguage(): SupportedLanguage {
  return normalizeLanguage(i18n.resolvedLanguage ?? i18n.language);
}

export { i18n };
