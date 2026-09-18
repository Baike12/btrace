-- Seed data for baike user's project
-- Project: cmqjq16yp0005luy3h55cr7hb, User: cmqjq16vf0000luy36l69nv19

BEGIN;

-- ============================================================================
-- TRACES (10 traces with varied data)
-- ============================================================================
INSERT INTO traces (id, timestamp, name, project_id, user_id, metadata, external_id, release, version, tags, input, output, session_id, bookmarked, public) VALUES
('trace-001', '2026-06-18 10:00:00', 'chat-completion', 'cmqjq16yp0005luy3h55cr7hb', 'user-42', '{"env": "production", "model": "gpt-4o"}', 'ext-001', 'v2.1.0', '1.0.0', ARRAY['production', 'chat', 'gpt-4o'], '{"messages": [{"role": "user", "content": "What is Langfuse?"}]}', '{"response": "Langfuse is an open-source LLM observability platform."}', 'session-chat-2026', true, false),
('trace-002', '2026-06-18 10:05:00', 'rag-query', 'cmqjq16yp0005luy3h55cr7hb', 'user-42', '{"env": "production", "pipeline": "rag-v3"}', 'ext-002', 'v2.1.0', '1.0.0', ARRAY['production', 'rag', 'embedding'], '{"query": "How does attention mechanism work?"}', '{"answer": "The attention mechanism computes weighted sums of values...", "sources": ["doc-1", "doc-2"]}', 'session-rag-2026', true, false),
('trace-003', '2026-06-18 10:10:00', 'image-generation', 'cmqjq16yp0005luy3h55cr7hb', 'user-77', '{"env": "production", "model": "dall-e-3"}', 'ext-003', 'v2.0.0', '2.0.1', ARRAY['production', 'image', 'dall-e'], '{"prompt": "A serene mountain landscape at sunset"}', '{"image_url": "https://cdn.example.com/gen/img-001.png"}', 'session-image-2026', false, false),
('trace-004', '2026-06-18 10:15:00', 'code-generation', 'cmqjq16yp0005luy3h55cr7hb', 'user-42', '{"env": "staging", "model": "claude-sonnet-4-6"}', 'ext-004', 'v2.1.0', '1.0.0', ARRAY['staging', 'code', 'claude'], '{"task": "Write a Python function to sort a list of dicts by key"}', '{"code": "def sort_dicts(lst, key):\\n    return sorted(lst, key=lambda x: x.get(key, \\\"\\\"))"}', 'session-code-2026', false, false),
('trace-005', '2026-06-18 10:20:00', 'chat-completion', 'cmqjq16yp0005luy3h55cr7hb', 'user-99', '{"env": "development", "model": "gpt-4o-mini"}', 'ext-005', 'v2.2.0-dev', '0.0.1', ARRAY['development', 'chat', 'gpt-4o-mini'], '{"messages": [{"role": "user", "content": "Explain recursion"}]}', '{"response": "Recursion is when a function calls itself..."}', 'session-chat-2026', false, false),
('trace-006', '2026-06-19 08:00:00', 'translation', 'cmqjq16yp0005luy3h55cr7hb', 'user-42', '{"env": "production", "model": "claude-fable-5", "target_lang": "zh"}', 'ext-006', 'v2.1.0', '1.0.0', ARRAY['production', 'translation'], '{"text": "Hello, how are you?", "source_lang": "en", "target_lang": "zh"}', '{"translated": "你好，你怎么样？"}', 'session-trans-2026', false, true),
('trace-007', '2026-06-19 08:30:00', 'sentiment-analysis', 'cmqjq16yp0005luy3h55cr7hb', 'user-77', '{"env": "production", "model": "gpt-4o"}', 'ext-007', 'v2.1.0', '1.0.0', ARRAY['production', 'nlp', 'sentiment'], '{"text": "I absolutely love this product! It has changed my life."}', '{"sentiment": "positive", "score": 0.98}', 'session-sent-2026', true, true),
('trace-008', '2026-06-19 09:00:00', 'summarization', 'cmqjq16yp0005luy3h55cr7hb', 'user-55', '{"env": "production", "model": "claude-haiku-4-5"}', 'ext-008', 'v2.1.0', '1.0.0', ARRAY['production', 'nlp', 'summary'], '{"text": "Lorem ipsum dolor sit amet... (long article about AI safety)", "max_length": 200}', '{"summary": "AI safety research focuses on ensuring advanced AI systems remain aligned with human values."}', 'session-sum-2026', false, false),
('trace-009', '2026-06-19 09:30:00', 'chat-completion', 'cmqjq16yp0005luy3h55cr7hb', 'user-42', '{"env": "production", "model": "gpt-4o"}', NULL, 'v2.1.0', '1.0.0', ARRAY['production', 'chat'], '{"messages": [{"role": "user", "content": "Write a haiku about programming"}]}', '{"response": "Code flows like water / Bugs hide in the darkest depths / Tests bring peace of mind"}', NULL, true, false),
('trace-010', '2026-06-19 10:00:00', 'error-case', 'cmqjq16yp0005luy3h55cr7hb', 'user-42', '{"env": "production", "model": "gpt-4o"}', 'ext-010', 'v2.1.0', '1.0.0', ARRAY['production', 'error', 'timeout'], '{"messages": [{"role": "user", "content": "Generate a very detailed report on AI trends 2026"}]}', '{"error": "Request timed out after 30s", "partial_response": null}', 'session-error-2026', false, false);

-- ============================================================================
-- OBSERVATIONS (30+ observations across different trace types)
-- ============================================================================
-- Trace 1 observations (chat-completion with RAG pipeline)
INSERT INTO observations (id, name, start_time, end_time, trace_id, project_id, type, level, model, input, output, metadata, prompt_tokens, completion_tokens, total_tokens, input_cost, output_cost, total_cost, internal_model, calculated_input_cost, calculated_output_cost, calculated_total_cost) VALUES
('obs-001', 'LLM Chat Completion', '2026-06-18 10:00:00', '2026-06-18 10:00:03.500', 'trace-001', 'cmqjq16yp0005luy3h55cr7hb', 'GENERATION', 'DEFAULT', 'gpt-4o', '{"messages": [{"role": "user", "content": "What is Langfuse?"}]}', '{"response": "Langfuse is an open-source LLM observability platform."}', '{"temperature": 0.7, "max_tokens": 500}', 150, 200, 350, 0.00075, 0.00200, 0.00275, 'openai/gpt-4o', 0.00075, 0.00200, 0.00275),
('obs-002', 'Chat Span', '2026-06-18 10:00:00', '2026-06-18 10:00:03.500', 'trace-001', 'cmqjq16yp0005luy3h55cr7hb', 'SPAN', 'DEFAULT', NULL, '{"step": "chat-handler"}', '{"status": "completed"}', '{"handler": "ChatHandler"}', 0, 0, 0, 0, 0, 0, NULL, 0, 0, 0);

-- Trace 2 observations (RAG pipeline: embedding -> retrieval -> generation)
INSERT INTO observations (id, name, start_time, end_time, trace_id, project_id, type, level, model, input, output, metadata, prompt_tokens, completion_tokens, total_tokens, input_cost, output_cost, total_cost, internal_model, calculated_input_cost, calculated_output_cost, calculated_total_cost) VALUES
('obs-003', 'Embedding Query', '2026-06-18 10:05:00', '2026-06-18 10:05:00.500', 'trace-002', 'cmqjq16yp0005luy3h55cr7hb', 'GENERATION', 'DEFAULT', 'text-embedding-3-small', '{"text": "How does attention mechanism work?"}', '{"embedding_dim": 1536, "token_count": 5}', '{"model": "text-embedding-3-small"}', 50, 0, 50, 0.00001, 0, 0.00001, 'openai/text-embedding-3-small', 0.00001, 0, 0.00001),
('obs-004', 'Vector Retrieval', '2026-06-18 10:05:00.500', '2026-06-18 10:05:00.800', 'trace-002', 'cmqjq16yp0005luy3h55cr7hb', 'SPAN', 'DEFAULT', NULL, '{"embeddings": [[0.1,0.2,0.3]], "top_k": 5}', '{"documents": [{"id": "doc-1", "score": 0.95}, {"id": "doc-2", "score": 0.87}]}', '{"index": "knowledge-base"}', 0, 0, 0, 0, 0, 0, NULL, 0, 0, 0),
('obs-005', 'RAG Generation', '2026-06-18 10:05:00.800', '2026-06-18 10:05:04.200', 'trace-002', 'cmqjq16yp0005luy3h55cr7hb', 'GENERATION', 'DEFAULT', 'gpt-4o', '{"query": "How does attention mechanism work?", "context": ["doc-1 content...", "doc-2 content..."]}', '{"answer": "The attention mechanism computes weighted sums of values..."}', '{"temperature": 0.3}', 800, 300, 1100, 0.00400, 0.00300, 0.00700, 'openai/gpt-4o', 0.00400, 0.00300, 0.00700);

-- Trace 3 observations (image generation)
INSERT INTO observations (id, name, start_time, end_time, trace_id, project_id, type, level, model, input, output, metadata, prompt_tokens, completion_tokens, total_tokens, input_cost, output_cost, total_cost, internal_model, calculated_input_cost, calculated_output_cost, calculated_total_cost) VALUES
('obs-006', 'Image Generation', '2026-06-18 10:10:00', '2026-06-18 10:10:08.000', 'trace-003', 'cmqjq16yp0005luy3h55cr7hb', 'GENERATION', 'DEFAULT', 'dall-e-3', '{"prompt": "A serene mountain landscape at sunset"}', '{"image_url": "https://cdn.example.com/gen/img-001.png"}', '{"size": "1024x1024", "quality": "hd"}', 100, 0, 100, 0.04000, 0, 0.04000, 'openai/dall-e-3', 0.04000, 0, 0.04000),
('obs-007', 'Image Post-Processing', '2026-06-18 10:10:08.000', '2026-06-18 10:10:10.000', 'trace-003', 'cmqjq16yp0005luy3h55cr7hb', 'SPAN', 'DEFAULT', NULL, '{"image_url": "https://cdn.example.com/gen/img-001.png", "filters": ["sharpen", "contrast"]}', '{"processed_url": "https://cdn.example.com/gen/img-001-final.png"}', '{"library": "pillow"}', 0, 0, 0, 0, 0, 0, NULL, 0, 0, 0);

-- Trace 4 observations (code generation with chain-of-thought)
INSERT INTO observations (id, name, start_time, end_time, trace_id, project_id, type, level, model, input, output, metadata, prompt_tokens, completion_tokens, total_tokens, input_cost, output_cost, total_cost, internal_model, calculated_input_cost, calculated_output_cost, calculated_total_cost) VALUES
('obs-008', 'Code Generation', '2026-06-18 10:15:00', '2026-06-18 10:15:05.000', 'trace-004', 'cmqjq16yp0005luy3h55cr7hb', 'GENERATION', 'DEFAULT', 'claude-sonnet-4-6', '{"task": "Write a Python function to sort a list of dicts by key"}', '{"code": "def sort_dicts(lst, key):\\n    return sorted(lst, key=lambda x: x.get(key, \\\"\\\"))"}', '{"language": "python", "max_tokens": 1000}', 200, 150, 350, 0.00060, 0.00150, 0.00210, 'anthropic/claude-sonnet-4-6', 0.00060, 0.00150, 0.00210),
('obs-009', 'Code Review', '2026-06-18 10:15:05.000', '2026-06-18 10:15:07.000', 'trace-004', 'cmqjq16yp0005luy3h55cr7hb', 'GENERATION', 'DEFAULT', 'claude-haiku-4-5', '{"code": "def sort_dicts...", "review_prompt": "Check for edge cases and type safety"}', '{"issues": [], "suggestions": ["Add type hints", "Handle None key values"]}', '{"type": "code-review"}', 180, 80, 260, 0.00004, 0.00008, 0.00012, 'anthropic/claude-haiku-4-5', 0.00004, 0.00008, 0.00012);

-- Trace 5 observations (simple chat)
INSERT INTO observations (id, name, start_time, end_time, trace_id, project_id, type, level, model, input, output, metadata, prompt_tokens, completion_tokens, total_tokens, input_cost, output_cost, total_cost, internal_model, calculated_input_cost, calculated_output_cost, calculated_total_cost) VALUES
('obs-010', 'Chat Completion', '2026-06-18 10:20:00', '2026-06-18 10:20:02.000', 'trace-005', 'cmqjq16yp0005luy3h55cr7hb', 'GENERATION', 'DEFAULT', 'gpt-4o-mini', '{"messages": [{"role": "user", "content": "Explain recursion"}]}', '{"response": "Recursion is when a function calls itself..."}', '{"temperature": 0.8}', 80, 200, 280, 0.00002, 0.00012, 0.00014, 'openai/gpt-4o-mini', 0.00002, 0.00012, 0.00014);

-- Trace 6 observations (translation pipeline)
INSERT INTO observations (id, name, start_time, end_time, trace_id, project_id, type, level, model, input, output, metadata, prompt_tokens, completion_tokens, total_tokens, input_cost, output_cost, total_cost, internal_model, calculated_input_cost, calculated_output_cost, calculated_total_cost) VALUES
('obs-011', 'Language Detection', '2026-06-19 08:00:00', '2026-06-19 08:00:00.300', 'trace-006', 'cmqjq16yp0005luy3h55cr7hb', 'SPAN', 'DEFAULT', NULL, '{"text": "Hello, how are you?"}', '{"detected_lang": "en", "confidence": 0.99}', '{"detector": "langdetect"}', 0, 0, 0, 0, 0, 0, NULL, 0, 0, 0),
('obs-012', 'Translation', '2026-06-19 08:00:00.300', '2026-06-19 08:00:03.000', 'trace-006', 'cmqjq16yp0005luy3h55cr7hb', 'GENERATION', 'DEFAULT', 'claude-fable-5', '{"text": "Hello, how are you?", "source_lang": "en", "target_lang": "zh"}', '{"translated": "你好，你怎么样？"}', '{"model": "claude-fable-5"}', 50, 30, 80, 0.00015, 0.00045, 0.00060, 'anthropic/claude-fable-5', 0.00015, 0.00045, 0.00060);

-- Trace 7 observations (sentiment analysis)
INSERT INTO observations (id, name, start_time, end_time, trace_id, project_id, type, level, model, input, output, metadata, prompt_tokens, completion_tokens, total_tokens, input_cost, output_cost, total_cost, internal_model, calculated_input_cost, calculated_output_cost, calculated_total_cost) VALUES
('obs-013', 'Sentiment Analysis', '2026-06-19 08:30:00', '2026-06-19 08:30:01.500', 'trace-007', 'cmqjq16yp0005luy3h55cr7hb', 'GENERATION', 'DEFAULT', 'gpt-4o', '{"text": "I absolutely love this product! It has changed my life."}', '{"sentiment": "positive", "score": 0.98, "reasoning": "Strong positive language with emotional impact words"}', '{"temperature": 0}', 60, 50, 110, 0.00030, 0.00050, 0.00080, 'openai/gpt-4o', 0.00030, 0.00050, 0.00080);

-- Trace 8 observations (summarization)
INSERT INTO observations (id, name, start_time, end_time, trace_id, project_id, type, level, model, input, output, metadata, prompt_tokens, completion_tokens, total_tokens, input_cost, output_cost, total_cost, internal_model, calculated_input_cost, calculated_output_cost, calculated_total_cost) VALUES
('obs-014', 'Text Chunking', '2026-06-19 09:00:00', '2026-06-19 09:00:00.200', 'trace-008', 'cmqjq16yp0005luy3h55cr7hb', 'SPAN', 'DEFAULT', NULL, '{"text": "... (long article)", "chunk_size": 2000, "overlap": 200}', '{"chunks": ["chunk-1: ...", "chunk-2: ...", "chunk-3: ..."]}', '{"method": "sliding-window"}', 0, 0, 0, 0, 0, 0, NULL, 0, 0, 0),
('obs-015', 'Summarize Chunks', '2026-06-19 09:00:00.200', '2026-06-19 09:00:04.000', 'trace-008', 'cmqjq16yp0005luy3h55cr7hb', 'GENERATION', 'DEFAULT', 'claude-haiku-4-5', '{"chunks": ["..."]}', '{"chunk_summaries": ["ch1 summary","ch2 summary","ch3 summary"]}', '{"temperature": 0.2}', 2000, 600, 2600, 0.00050, 0.00060, 0.00110, 'anthropic/claude-haiku-4-5', 0.00050, 0.00060, 0.00110),
('obs-016', 'Final Summary Merge', '2026-06-19 09:00:04.000', '2026-06-19 09:00:05.500', 'trace-008', 'cmqjq16yp0005luy3h55cr7hb', 'GENERATION', 'DEFAULT', 'claude-fable-5', '{"chunk_summaries": ["ch1 summary","ch2 summary","ch3 summary"]}', '{"summary": "AI safety research focuses on ensuring advanced AI systems remain aligned with human values."}', '{"temperature": 0.1}', 800, 200, 1000, 0.00240, 0.00300, 0.00540, 'anthropic/claude-fable-5', 0.00240, 0.00300, 0.00540);

-- Trace 9 observations (chat with tool calling)
INSERT INTO observations (id, name, start_time, end_time, trace_id, project_id, type, level, model, input, output, metadata, prompt_tokens, completion_tokens, total_tokens, input_cost, output_cost, total_cost, internal_model, calculated_input_cost, calculated_output_cost, calculated_total_cost) VALUES
('obs-017', 'Tool: Calendar Lookup', '2026-06-19 09:30:00', '2026-06-19 09:30:00.100', 'trace-009', 'cmqjq16yp0005luy3h55cr7hb', 'SPAN', 'DEFAULT', NULL, '{"tool": "calendar", "action": "lookup", "date": "2026-06-19"}', '{"events": ["10:00 Standup", "14:00 Sprint Review"]}', '{"tool_type": "api"}', 0, 0, 0, 0, 0, 0, NULL, 0, 0, 0),
('obs-018', 'Chat with Context', '2026-06-19 09:30:00.100', '2026-06-19 09:30:03.000', 'trace-009', 'cmqjq16yp0005luy3h55cr7hb', 'GENERATION', 'DEFAULT', 'gpt-4o', '{"messages": [{"role": "user", "content": "Write a haiku about programming"}], "context": {"events": ["10:00 Standup"]}}', '{"response": "Code flows like water / Bugs hide in the darkest depths / Tests bring peace of mind"}', '{"temperature": 0.9, "tools": ["calendar"]}', 120, 60, 180, 0.00060, 0.00060, 0.00120, 'openai/gpt-4o', 0.00060, 0.00060, 0.00120);

-- Trace 10 observations (error case)
INSERT INTO observations (id, name, start_time, end_time, trace_id, project_id, type, level, model, input, output, metadata, prompt_tokens, completion_tokens, total_tokens, input_cost, output_cost, total_cost, status_message, internal_model, calculated_input_cost, calculated_output_cost, calculated_total_cost) VALUES
('obs-019', 'Report Generation Attempt', '2026-06-19 10:00:00', '2026-06-19 10:00:30.000', 'trace-010', 'cmqjq16yp0005luy3h55cr7hb', 'GENERATION', 'ERROR', 'gpt-4o', '{"messages": [{"role": "user", "content": "Generate a very detailed report on AI trends 2026"}]}', '{"error": "Request timed out after 30s", "partial_response": null}', '{"temperature": 0.5, "max_tokens": 8000, "timeout": 30000}', 5000, 0, 5000, 0.02500, 0, 0.02500, 'Request timed out after 30s', 'openai/gpt-4o', 0.02500, 0, 0.02500);

-- Nested observation: obs-008 has a child chain-of-thought step
INSERT INTO observations (id, name, start_time, end_time, trace_id, project_id, type, level, parent_observation_id, input, output, metadata) VALUES
('obs-020', 'Chain of Thought', '2026-06-18 10:15:00.500', '2026-06-18 10:15:03.000', 'trace-004', 'cmqjq16yp0005luy3h55cr7hb', 'SPAN', 'DEFAULT', 'obs-008', '{"task": "sort dicts by key"}', '{"thoughts": ["Understand input structure", "Use sorted() with key function", "Handle missing keys gracefully"]}', '{"type": "reasoning"}');

-- ============================================================================
-- SCORES (scores for both traces and observations)
-- ============================================================================
INSERT INTO scores (id, timestamp, name, value, observation_id, trace_id, comment, source, project_id, data_type, string_value) VALUES
-- Trace-level scores
('score-001', '2026-06-18 10:00:05', 'user_feedback', 4.5, NULL, 'trace-001', 'Great response, very informative', 'API', 'cmqjq16yp0005luy3h55cr7hb', 'NUMERIC', NULL),
('score-002', '2026-06-18 10:00:05', 'latency_ms', 3500, NULL, 'trace-001', NULL, 'API', 'cmqjq16yp0005luy3h55cr7hb', 'NUMERIC', NULL),
('score-003', '2026-06-18 10:05:06', 'relevance', 0.92, NULL, 'trace-002', 'Highly relevant answer with good citations', 'API', 'cmqjq16yp0005luy3h55cr7hb', 'NUMERIC', NULL),
('score-004', '2026-06-18 10:05:06', 'faithfulness', 0.88, NULL, 'trace-002', 'Minor factual discrepancy in doc-2 citation', 'API', 'cmqjq16yp0005luy3h55cr7hb', 'NUMERIC', NULL),
('score-005', '2026-06-18 10:10:10', 'image_quality', 4.8, NULL, 'trace-003', 'Stunning image, great composition', 'ANNOTATION', 'cmqjq16yp0005luy3h55cr7hb', 'NUMERIC', NULL),
('score-006', '2026-06-18 10:15:07', 'code_quality', 4.2, NULL, 'trace-004', 'Good code, needs type hints', 'API', 'cmqjq16yp0005luy3h55cr7hb', 'NUMERIC', NULL),
('score-007', '2026-06-18 10:20:02', 'clarity', 3.5, NULL, 'trace-005', 'Response could be more detailed', 'API', 'cmqjq16yp0005luy3h55cr7hb', 'NUMERIC', NULL),
('score-008', '2026-06-19 08:00:03', 'translation_quality', 4.7, NULL, 'trace-006', 'Accurate and natural-sounding translation', 'ANNOTATION', 'cmqjq16yp0005luy3h55cr7hb', 'NUMERIC', NULL),
('score-009', '2026-06-19 08:30:02', 'sentiment_accuracy', 0.98, NULL, 'trace-007', 'Correctly identified strong positive sentiment', 'API', 'cmqjq16yp0005luy3h55cr7hb', 'NUMERIC', NULL),
('score-010', '2026-06-19 09:00:06', 'summary_quality', 4.0, NULL, 'trace-008', 'Good summary but missed some key points', 'API', 'cmqjq16yp0005luy3h55cr7hb', 'NUMERIC', NULL),
('score-011', '2026-06-19 09:30:03', 'creativity', 4.8, NULL, 'trace-009', 'Beautiful haiku, perfect imagery', 'ANNOTATION', 'cmqjq16yp0005luy3h55cr7hb', 'NUMERIC', NULL),
-- Observation-level scores
('score-012', '2026-06-18 10:00:04', 'toxicity', 0.01, 'obs-001', 'trace-001', 'No toxic content detected', 'API', 'cmqjq16yp0005luy3h55cr7hb', 'NUMERIC', NULL),
('score-013', '2026-06-18 10:05:05', 'hallucination', 0.05, 'obs-005', 'trace-002', 'Very low hallucination score', 'API', 'cmqjq16yp0005luy3h55cr7hb', 'NUMERIC', NULL),
('score-014', '2026-06-18 10:15:06', 'code_security', 0.95, 'obs-008', 'trace-004', 'No security issues found', 'API', 'cmqjq16yp0005luy3h55cr7hb', 'NUMERIC', NULL),
-- Categorical scores
('score-015', '2026-06-19 10:00:31', 'error_severity', NULL, 'obs-019', 'trace-010', 'Timeout occurred during generation', 'API', 'cmqjq16yp0005luy3h55cr7hb', 'CATEGORICAL', 'high'),
('score-016', '2026-06-19 08:30:02', 'sentiment_label', NULL, NULL, 'trace-007', NULL, 'API', 'cmqjq16yp0005luy3h55cr7hb', 'CATEGORICAL', 'positive'),
('score-017', '2026-06-18 10:05:06', 'rag_category', NULL, NULL, 'trace-002', 'RAG pipeline performed well', 'API', 'cmqjq16yp0005luy3h55cr7hb', 'CATEGORICAL', 'success'),
-- Boolean score
('score-018', '2026-06-19 08:00:03', 'needs_review', NULL, NULL, 'trace-006', 'Translation approved by reviewer', 'ANNOTATION', 'cmqjq16yp0005luy3h55cr7hb', 'BOOLEAN', 'false');

COMMIT;
