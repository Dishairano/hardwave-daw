// Imported first in main.tsx: React looks for the commit hook when
// react-dom loads, so the FPS meter's render count has to be in place
// before that.
import { installCommitCounter } from './frameStats'

installCommitCounter()
