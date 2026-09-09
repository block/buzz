import { Link } from "@tanstack/react-router";
import { baseUiBackingSentence } from "./baseUiBackingSentence";

/** Inline inheritance note; links point to the actual owning library. */
export function BaseUiBackingLine({ slug }: { slug: string }) {
  const segments = baseUiBackingSentence(slug);
  if (!segments.length) return null;
  return (
    <>
      {" "}
      {segments.map((segment, index) =>
        segment.kind === "text" ? (
          // biome-ignore lint/suspicious/noArrayIndexKey: static sentence segments retain their order
          <span key={index}>{segment.value}</span>
        ) : segment.external ? (
          <a
            key={segment.href}
            href={segment.href}
            target="_blank"
            rel="noreferrer"
            className="text-purple-12 underline decoration-1 underline-offset-2"
          >
            {segment.value}
          </a>
        ) : (
          <Link
            key={segment.href}
            to={segment.href}
            className="text-purple-12 underline decoration-1 underline-offset-2"
          >
            {segment.value}
          </Link>
        ),
      )}
    </>
  );
}
