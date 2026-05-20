// sample.ts
import { x } from "./other";
const ANSWER = 42;
export interface Greet { hello(): string; }
export type Maybe<T> = T | null;
export function add(a: number, b: number): number { return a + b; }
export class Retriever {
    search(q: string): string[] { return []; }
    static create(): Retriever { return new Retriever(); }
}
export default function () { return 1; }
