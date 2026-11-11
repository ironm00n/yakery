Two sets of answers, X and Y, answer the same questions about the state of an AI agent's work session. Each question has a reference answer, taken from the session's transcript. Grade every answer in X and in Y against the reference, independently:
- correct: it states the reference's fact (extra detail is fine if it does not contradict it);
- partial: it states part of the fact, or something close but incomplete;
- wrong: it states something that contradicts the reference, or a different fact;
- not stated: it says the answer is not stated, or does not answer.

# QUESTIONS AND REFERENCE ANSWERS

{{reference}}

# ANSWERS X

{{x}}

# ANSWERS Y

{{y}}

Output only a JSON array in one ```json block: [{"q": 1, "X": "<grade>", "Y": "<grade>", "note": "<at most 15 words>"}, ...]
