/** Assemble des noms de classes en ignorant les valeurs fausses. */
export function cx(...names: (string | false | null | undefined)[]): string {
  return names.filter(Boolean).join(" ");
}
