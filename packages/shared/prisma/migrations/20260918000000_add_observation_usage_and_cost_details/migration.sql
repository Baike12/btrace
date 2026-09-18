-- Add Langfuse v4 OTel usage/cost breakdown columns.
--
-- usage_details carries the raw `langfuse.observation.usage_details` span attribute
-- payload keyed by usage type (input / output / total / cache_read_input_tokens /
-- cache_creation_input_tokens / cache_miss_input_tokens). The existing
-- prompt_tokens / completion_tokens / total_tokens columns stay as derived
-- summaries of the same payload.
--
-- cost_details mirrors usage_details, keyed by the same usage types.
--
-- Both are nullable, so this migration is additive and non-breaking.

ALTER TABLE "observations" ADD COLUMN "usage_details" JSONB;
ALTER TABLE "observations" ADD COLUMN "cost_details" JSONB;
