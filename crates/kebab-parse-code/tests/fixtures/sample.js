// sample.js
import { x } from "./other";
const ANSWER = 42;
export function add(a, b) { return a + b; }
export class Retriever {
    search(q) { return []; }
    static create() { return new Retriever(); }
}
export default function () { return 1; }
