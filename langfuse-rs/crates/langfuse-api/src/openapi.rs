use serde_json::{json, Value};

/// Build OpenAPI 3.0 JSON spec manually (avoids derive macro compatibility issues)
pub fn build_openapi_spec() -> Value {
    json!({
        "openapi": "3.0.3",
        "info": {
            "title": "Langfuse API",
            "version": "0.1.0",
            "description": "Langfuse PG-Only Fork — Rust Backend REST API"
        },
        "servers": [
            { "url": "http://localhost:8080", "description": "Local dev server" }
        ],
        "security": [{ "bearer_auth": [] }],
        "components": {
            "securitySchemes": {
                "bearer_auth": {
                    "type": "http",
                    "scheme": "bearer",
                    "bearerFormat": "JWT"
                }
            },
            "schemas": {
                "Trace": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "project_id": { "type": "string" },
                        "external_id": { "type": "string", "nullable": true },
                        "timestamp": { "type": "string", "format": "date-time" },
                        "name": { "type": "string", "nullable": true },
                        "user_id": { "type": "string", "nullable": true },
                        "metadata": { "nullable": true },
                        "release": { "type": "string", "nullable": true },
                        "version": { "type": "string", "nullable": true },
                        "public": { "type": "boolean" },
                        "bookmarked": { "type": "boolean" },
                        "tags": { "type": "array", "items": { "type": "string" } },
                        "input": { "nullable": true },
                        "output": { "nullable": true },
                        "session_id": { "type": "string", "nullable": true },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "Observation": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "trace_id": { "type": "string", "nullable": true },
                        "project_id": { "type": "string" },
                        "type": { "type": "string" },
                        "start_time": { "type": "string", "format": "date-time", "nullable": true },
                        "end_time": { "type": "string", "format": "date-time", "nullable": true },
                        "name": { "type": "string", "nullable": true },
                        "metadata": { "nullable": true },
                        "parent_observation_id": { "type": "string", "nullable": true },
                        "level": { "type": "string", "nullable": true },
                        "status_message": { "type": "string", "nullable": true },
                        "version": { "type": "string", "nullable": true },
                        "model": { "type": "string", "nullable": true },
                        "internal_model": { "type": "string", "nullable": true },
                        "model_parameters": { "nullable": true },
                        "input": { "nullable": true },
                        "output": { "nullable": true },
                        "prompt_tokens": { "type": "integer", "nullable": true },
                        "completion_tokens": { "type": "integer", "nullable": true },
                        "total_tokens": { "type": "integer", "nullable": true },
                        "unit": { "type": "string", "nullable": true },
                        "input_cost": { "type": "number", "nullable": true },
                        "output_cost": { "type": "number", "nullable": true },
                        "total_cost": { "type": "number", "nullable": true },
                        "completion_start_time": { "type": "string", "format": "date-time", "nullable": true },
                        "prompt_id": { "type": "string", "nullable": true },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "Score": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "project_id": { "type": "string" },
                        "timestamp": { "type": "string", "format": "date-time" },
                        "name": { "type": "string", "nullable": true },
                        "value": { "type": "number", "nullable": true },
                        "source": { "type": "string" },
                        "author_user_id": { "type": "string", "nullable": true },
                        "comment": { "type": "string", "nullable": true },
                        "trace_id": { "type": "string", "nullable": true },
                        "observation_id": { "type": "string", "nullable": true },
                        "config_id": { "type": "string", "nullable": true },
                        "string_value": { "type": "string", "nullable": true },
                        "data_type": { "type": "string" },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "Session": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "project_id": { "type": "string" },
                        "environment": { "type": "string", "nullable": true },
                        "bookmarked": { "type": "boolean" },
                        "public": { "type": "boolean" },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "Dataset": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "project_id": { "type": "string" },
                        "name": { "type": "string" },
                        "description": { "type": "string", "nullable": true },
                        "metadata": { "nullable": true },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "PaginationMeta": {
                    "type": "object",
                    "properties": {
                        "cursor": { "type": "string", "nullable": true },
                        "has_more": { "type": "boolean" },
                        "total": { "type": "integer", "nullable": true }
                    }
                },
                "LoginRequest": {
                    "type": "object",
                    "required": ["email", "password"],
                    "properties": {
                        "email": { "type": "string", "format": "email" },
                        "password": { "type": "string" }
                    }
                },
                "LoginResponse": {
                    "type": "object",
                    "properties": {
                        "token": { "type": "string" },
                        "user": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "email": { "type": "string", "nullable": true },
                                "name": { "type": "string", "nullable": true },
                                "admin": { "type": "boolean" }
                            }
                        }
                    }
                }
            }
        },
        "paths": {
            "/api/auth/login": {
                "post": {
                    "tags": ["Auth"],
                    "summary": "Login with email and password",
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/LoginRequest" } } }
                    },
                    "responses": {
                        "200": { "description": "OK", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/LoginResponse" } } } },
                        "401": { "description": "Invalid credentials" }
                    }
                }
            },
            "/api/projects": {
                "get": {
                    "tags": ["Projects"],
                    "summary": "List projects for current user",
                    "security": [{ "bearer_auth": [] }],
                    "responses": {
                        "200": { "description": "OK" },
                        "401": { "description": "Unauthorized" }
                    }
                }
            },
            "/api/organizations": {
                "get": {
                    "tags": ["Organizations"],
                    "summary": "List organizations for current user",
                    "security": [{ "bearer_auth": [] }],
                    "responses": {
                        "200": { "description": "OK" },
                        "401": { "description": "Unauthorized" }
                    }
                }
            },
            "/api/traces": {
                "get": {
                    "tags": ["Traces"],
                    "summary": "List traces with cursor pagination",
                    "security": [{ "bearer_auth": [] }],
                    "parameters": [
                        { "name": "project_id", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "cursor", "in": "query", "required": false, "schema": { "type": "string" } },
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer" } }
                    ],
                    "responses": {
                        "200": { "description": "OK" },
                        "401": { "description": "Unauthorized" },
                        "403": { "description": "Forbidden" }
                    }
                }
            },
            "/api/traces/{trace_id}": {
                "get": {
                    "tags": ["Traces"],
                    "summary": "Get single trace by ID",
                    "security": [{ "bearer_auth": [] }],
                    "parameters": [
                        { "name": "trace_id", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "project_id", "in": "query", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": {
                        "200": { "description": "OK" },
                        "401": { "description": "Unauthorized" },
                        "403": { "description": "Forbidden" },
                        "404": { "description": "Not found" }
                    }
                }
            },
            "/api/observations": {
                "get": {
                    "tags": ["Observations"],
                    "summary": "List observations with optional filters",
                    "security": [{ "bearer_auth": [] }],
                    "parameters": [
                        { "name": "project_id", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "trace_id", "in": "query", "required": false, "schema": { "type": "string" } },
                        { "name": "type", "in": "query", "required": false, "schema": { "type": "string" } },
                        { "name": "cursor", "in": "query", "required": false, "schema": { "type": "string" } },
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer" } }
                    ],
                    "responses": {
                        "200": { "description": "OK" },
                        "401": { "description": "Unauthorized" },
                        "403": { "description": "Forbidden" }
                    }
                }
            },
            "/api/scores": {
                "get": {
                    "tags": ["Scores"],
                    "summary": "List scores with optional trace_id filter",
                    "security": [{ "bearer_auth": [] }],
                    "parameters": [
                        { "name": "project_id", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "trace_id", "in": "query", "required": false, "schema": { "type": "string" } },
                        { "name": "cursor", "in": "query", "required": false, "schema": { "type": "string" } },
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer" } }
                    ],
                    "responses": {
                        "200": { "description": "OK" },
                        "401": { "description": "Unauthorized" },
                        "403": { "description": "Forbidden" }
                    }
                }
            },
            "/api/sessions": {
                "get": {
                    "tags": ["Sessions"],
                    "summary": "List sessions for a project",
                    "security": [{ "bearer_auth": [] }],
                    "parameters": [
                        { "name": "project_id", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer" } }
                    ],
                    "responses": {
                        "200": { "description": "OK" },
                        "401": { "description": "Unauthorized" },
                        "403": { "description": "Forbidden" }
                    }
                }
            },
            "/api/datasets": {
                "get": {
                    "tags": ["Datasets"],
                    "summary": "List datasets for a project",
                    "security": [{ "bearer_auth": [] }],
                    "parameters": [
                        { "name": "project_id", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer" } }
                    ],
                    "responses": {
                        "200": { "description": "OK" },
                        "401": { "description": "Unauthorized" },
                        "403": { "description": "Forbidden" }
                    }
                }
            },
            "/api/prompts": {
                "get": {
                    "tags": ["Prompts"],
                    "summary": "List prompts for a project",
                    "security": [{ "bearer_auth": [] }],
                    "parameters": [
                        { "name": "project_id", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer" } }
                    ],
                    "responses": {
                        "200": { "description": "OK" },
                        "401": { "description": "Unauthorized" },
                        "403": { "description": "Forbidden" }
                    }
                }
            },
            "/api/models": {
                "get": {
                    "tags": ["Models"],
                    "summary": "List models for a project",
                    "security": [{ "bearer_auth": [] }],
                    "parameters": [
                        { "name": "project_id", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer" } }
                    ],
                    "responses": {
                        "200": { "description": "OK" },
                        "401": { "description": "Unauthorized" },
                        "403": { "description": "Forbidden" }
                    }
                }
            },
            "/api/evals": {
                "get": {
                    "tags": ["Evals"],
                    "summary": "List eval templates for a project",
                    "security": [{ "bearer_auth": [] }],
                    "parameters": [
                        { "name": "project_id", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer" } }
                    ],
                    "responses": {
                        "200": { "description": "OK" },
                        "401": { "description": "Unauthorized" },
                        "403": { "description": "Forbidden" }
                    }
                }
            },
            "/api/dashboards": {
                "get": {
                    "tags": ["Dashboards"],
                    "summary": "List dashboards for a project",
                    "security": [{ "bearer_auth": [] }],
                    "parameters": [
                        { "name": "project_id", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer" } }
                    ],
                    "responses": {
                        "200": { "description": "OK" },
                        "401": { "description": "Unauthorized" },
                        "403": { "description": "Forbidden" }
                    }
                }
            },
            "/api/comments": {
                "get": {
                    "tags": ["Comments"],
                    "summary": "List comments for a project",
                    "security": [{ "bearer_auth": [] }],
                    "parameters": [
                        { "name": "project_id", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer" } }
                    ],
                    "responses": {
                        "200": { "description": "OK" },
                        "401": { "description": "Unauthorized" },
                        "403": { "description": "Forbidden" }
                    }
                }
            },
            "/api/media": {
                "get": {
                    "tags": ["Media"],
                    "summary": "List media for a project",
                    "security": [{ "bearer_auth": [] }],
                    "parameters": [
                        { "name": "project_id", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer" } }
                    ],
                    "responses": {
                        "200": { "description": "OK" },
                        "401": { "description": "Unauthorized" },
                        "403": { "description": "Forbidden" }
                    }
                }
            },
            "/api/automations": {
                "get": {
                    "tags": ["Automations"],
                    "summary": "List automations for a project",
                    "security": [{ "bearer_auth": [] }],
                    "parameters": [
                        { "name": "project_id", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer" } }
                    ],
                    "responses": {
                        "200": { "description": "OK" },
                        "401": { "description": "Unauthorized" },
                        "403": { "description": "Forbidden" }
                    }
                }
            },
            "/api/monitors": {
                "get": {
                    "tags": ["Monitors"],
                    "summary": "List monitors for a project",
                    "security": [{ "bearer_auth": [] }],
                    "parameters": [
                        { "name": "project_id", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer" } }
                    ],
                    "responses": {
                        "200": { "description": "OK" },
                        "401": { "description": "Unauthorized" },
                        "403": { "description": "Forbidden" }
                    }
                }
            },
            "/api/users": {
                "get": {
                    "tags": ["Users"],
                    "summary": "List users (admin only)",
                    "security": [{ "bearer_auth": [] }],
                    "parameters": [
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer" } }
                    ],
                    "responses": {
                        "200": { "description": "OK" },
                        "401": { "description": "Unauthorized" },
                        "403": { "description": "Forbidden" }
                    }
                }
            }
        }
    })
}
