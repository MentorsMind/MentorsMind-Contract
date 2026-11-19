#![no_std]

use shared::Pagination;
use shared::ReentrancyGuard;
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, vec, Address, Bytes, BytesN, Env, IntoVal,
    Symbol, Vec,
};

const ADMIN: Symbol = symbol_short!("ADMIN");
const TOKEN: Symbol = symbol_short!("TOKEN");
const POST_COUNT: Symbol = symbol_short!("POST_CNT");
const CATEGORY_COUNT: Symbol = symbol_short!("CAT_CNT");
const REPUTATION_THRESHOLD: Symbol = symbol_short!("REP_THR");
const MIN_POST_STAKE: Symbol = symbol_short!("MIN_STAKE");
const MODERATION_THRESHOLD: Symbol = symbol_short!("MOD_THR");

const DEFAULT_REPUTATION_THRESHOLD: i128 = 10;
const DEFAULT_MIN_POST_STAKE: i128 = 1_000_000; // 0.001 MNT
const DEFAULT_MODERATION_THRESHOLD: u32 = 3;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PostType {
    Question,
    Discussion,
    Announcement,
    Resource,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PostStatus {
    Active,
    Hidden,
    Locked,
    Deleted,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModerationAction {
    Hide,
    Lock,
    Delete,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Post {
    pub id: u32,
    pub author: Address,
    pub title: Bytes,
    pub content_hash: BytesN<32>,
    pub post_type: PostType,
    pub category_id: u32,
    pub status: PostStatus,
    pub created_at: u64,
    pub updated_at: u64,
    pub stake_amount: i128,
    pub upvotes: i128,
    pub downvotes: i128,
    pub reply_count: u32,
    pub moderation_count: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reply {
    pub id: u32,
    pub post_id: u32,
    pub author: Address,
    pub content_hash: BytesN<32>,
    pub created_at: u64,
    pub updated_at: u64,
    pub stake_amount: i128,
    pub upvotes: i128,
    pub downvotes: i128,
    pub moderation_count: u32,
    pub is_best_answer: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Category {
    pub id: u32,
    pub name: Bytes,
    pub description: Bytes,
    pub created_at: u64,
    pub post_count: u32,
    pub is_active: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserReputation {
    pub address: Address,
    pub reputation: i128,
    pub posts_created: u32,
    pub replies_created: u32,
    pub upvotes_received: i128,
    pub downvotes_received: i128,
    pub moderation_actions: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForumStats {
    pub total_posts: u32,
    pub total_replies: u32,
    pub active_posts: u32,
    pub total_staked_mnt: i128,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Post(u32),
    Reply(u32),
    Category(u32),
    UserReputation(Address),
    Vote(u32, Address),      // For posts
    ReplyVote(u32, Address), // For replies
    Moderation(u32, Address),
    CategoryPost(u32, u32), // category_id, post_id
    UserPost(Address, u32),
    UserReply(Address, u32),
    TotalPosts,
    TotalReplies,
    ActivePosts,
    TotalStakedMnt,
    /// Ordinal index into a category's posts: (category_id, ordinal) -> post_id.
    CategoryIndex(u32, u32),
}

#[contract]
pub struct ForumContract;

#[contractimpl]
impl ForumContract {
    pub fn initialize(
        env: Env,
        admin: Address,
        mnt_token: Address,
        reputation_threshold: Option<i128>,
        min_post_stake: Option<i128>,
        moderation_threshold: Option<u32>,
    ) {
        if env.storage().persistent().has(&ADMIN) {
            panic!("already initialized");
        }

        let rep_threshold = reputation_threshold.unwrap_or(DEFAULT_REPUTATION_THRESHOLD);
        let min_stake = min_post_stake.unwrap_or(DEFAULT_MIN_POST_STAKE);
        let mod_threshold = moderation_threshold.unwrap_or(DEFAULT_MODERATION_THRESHOLD);

        if rep_threshold < 0 {
            panic!("invalid reputation threshold");
        }
        if min_stake < 0 {
            panic!("invalid minimum stake");
        }
        if mod_threshold == 0 {
            panic!("invalid moderation threshold");
        }

        env.storage().persistent().set(&ADMIN, &admin);
        env.storage().persistent().set(&TOKEN, &mnt_token);
        env.storage()
            .persistent()
            .set(&REPUTATION_THRESHOLD, &rep_threshold);
        env.storage().persistent().set(&MIN_POST_STAKE, &min_stake);
        env.storage()
            .persistent()
            .set(&MODERATION_THRESHOLD, &mod_threshold);
        env.storage().persistent().set(&POST_COUNT, &0u32);
        env.storage().persistent().set(&CATEGORY_COUNT, &0u32);
        env.storage().persistent().set(&DataKey::TotalPosts, &0u32);
        env.storage().persistent().set(&DataKey::TotalReplies, &0u32);
        env.storage().persistent().set(&DataKey::ActivePosts, &0u32);
        env.storage().persistent().set(&DataKey::TotalStakedMnt, &0i128);

        // Create default categories
        Self::create_default_categories(&env);
    }

    pub fn create_post(
        env: Env,
        author: Address,
        title: Bytes,
        content_hash: BytesN<32>,
        post_type: PostType,
        category_id: u32,
        stake_amount: i128,
    ) -> u32 {
        // Prevent reentrancy via the staked token transfer below.
        let _guard = ReentrancyGuard::enter(&env, symbol_short!("FORUM"));

        author.require_auth();

        let min_stake: i128 = env
            .storage()
            .persistent()
            .get(&MIN_POST_STAKE)
            .unwrap_or(DEFAULT_MIN_POST_STAKE);
        if stake_amount < min_stake {
            panic!("stake amount below minimum");
        }

        // Check if category exists and is active
        let category = Self::get_category(env.clone(), category_id);
        if !category.is_active {
            panic!("category not active");
        }
        let category_index: u32 = category.post_count;

        // Check reputation threshold for certain post types
        let rep_threshold: i128 = env
            .storage()
            .persistent()
            .get(&REPUTATION_THRESHOLD)
            .unwrap_or(DEFAULT_REPUTATION_THRESHOLD);
        match post_type {
            PostType::Announcement => {
                let user_rep = Self::get_user_reputation(env.clone(), author.clone()).reputation;
                if user_rep < rep_threshold * 10 {
                    panic!("insufficient reputation for announcements");
                }
            }
            PostType::Resource => {
                let user_rep = Self::get_user_reputation(env.clone(), author.clone()).reputation;
                if user_rep < rep_threshold * 5 {
                    panic!("insufficient reputation for resources");
                }
            }
            _ => {}
        }

        // Transfer stake tokens to contract
        Self::transfer_tokens(&env, &author, &env.current_contract_address(), stake_amount);

        let mut count: u32 = env.storage().persistent().get(&POST_COUNT).unwrap_or(0);
        count = count.checked_add(1).expect("post count overflow");

        let now = env.ledger().timestamp();

        let post_type_clone = post_type.clone();
        let post = Post {
            id: count,
            author: author.clone(),
            title,
            content_hash,
            post_type,
            category_id,
            status: PostStatus::Active,
            created_at: now,
            updated_at: now,
            stake_amount,
            upvotes: 0,
            downvotes: 0,
            reply_count: 0,
            moderation_count: 0,
        };

        env.storage().persistent().set(&POST_COUNT, &count);
        env.storage().persistent().set(&DataKey::Post(count), &post);
        env.storage()
            .persistent()
            .set(&DataKey::CategoryPost(category_id, count), &true);
        env.storage()
            .persistent()
            .set(&DataKey::CategoryIndex(category_id, category_index), &count);
        env.storage()
            .persistent()
            .set(&DataKey::UserPost(author.clone(), count), &true);
        let total_posts = env.storage().persistent().get(&DataKey::TotalPosts).unwrap_or(0u32);
        env.storage().persistent().set(&DataKey::TotalPosts, &total_posts.checked_add(1).expect("total posts overflow"));
        let active_posts = env.storage().persistent().get(&DataKey::ActivePosts).unwrap_or(0u32);
        env.storage().persistent().set(&DataKey::ActivePosts, &active_posts.checked_add(1).expect("active posts overflow"));
        let total_staked = env.storage().persistent().get(&DataKey::TotalStakedMnt).unwrap_or(0i128);
        env.storage().persistent().set(&DataKey::TotalStakedMnt, &total_staked.checked_add(stake_amount).expect("total stake overflow"));

        // Update user reputation
        Self::update_user_reputation(&env, &author, |rep| UserReputation {
            posts_created: rep.posts_created + 1,
            ..rep
        });

        // Update category post count
        Self::update_category_post_count(&env, category_id, 1);

        env.events().publish(
            (symbol_short!("FRM"), symbol_short!("POST_CR"), count),
            (author, post_type_clone, category_id, stake_amount),
        );

        count
    }

    pub fn create_reply(
        env: Env,
        author: Address,
        post_id: u32,
        content_hash: BytesN<32>,
        stake_amount: i128,
    ) -> u32 {
        // Prevent reentrancy via the staked token transfer below.
        let _guard = ReentrancyGuard::enter(&env, symbol_short!("FORUM"));

        author.require_auth();

        let post = Self::get_post(env.clone(), post_id);
        if post.status != PostStatus::Active {
            panic!("post not active");
        }

        let min_stake: i128 = env
            .storage()
            .persistent()
            .get(&MIN_POST_STAKE)
            .unwrap_or(DEFAULT_MIN_POST_STAKE);
        if stake_amount < min_stake {
            panic!("stake amount below minimum");
        }

        // Transfer stake tokens to contract
        Self::transfer_tokens(&env, &author, &env.current_contract_address(), stake_amount);

        let reply_id = post
            .reply_count
            .checked_add(1)
            .expect("reply count overflow");
        let now = env.ledger().timestamp();

        let reply = Reply {
            id: reply_id,
            post_id,
            author: author.clone(),
            content_hash,
            created_at: now,
            updated_at: now,
            stake_amount,
            upvotes: 0,
            downvotes: 0,
            moderation_count: 0,
            is_best_answer: false,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Reply(reply_id), &reply);
        env.storage()
            .persistent()
            .set(&DataKey::UserReply(author.clone(), reply_id), &true);

        // Update post reply count
        let mut updated_post = post;
        updated_post.reply_count = reply_id;
        updated_post.updated_at = now;
        env.storage()
            .persistent()
            .set(&DataKey::Post(post_id), &updated_post);
        let total_replies = env.storage().persistent().get(&DataKey::TotalReplies).unwrap_or(0u32);
        env.storage().persistent().set(&DataKey::TotalReplies, &total_replies.checked_add(1).expect("total replies overflow"));
        let total_staked = env.storage().persistent().get(&DataKey::TotalStakedMnt).unwrap_or(0i128);
        env.storage().persistent().set(&DataKey::TotalStakedMnt, &total_staked.checked_add(stake_amount).expect("total stake overflow"));

        // Update user reputation
        Self::update_user_reputation(&env, &author, |rep| UserReputation {
            replies_created: rep.replies_created + 1,
            ..rep
        });

        env.events().publish(
            (symbol_short!("FRM"), symbol_short!("REPLY_CR"), reply_id),
            (author, post_id, stake_amount),
        );

        reply_id
    }

    pub fn vote_post(env: Env, voter: Address, post_id: u32, upvote: bool) {
        voter.require_auth();

        let mut post = Self::get_post(env.clone(), post_id);
        if post.status != PostStatus::Active {
            panic!("post not active");
        }

        let vote_key = DataKey::Vote(post_id, voter.clone());
        if env.storage().persistent().has(&vote_key) {
            panic!("already voted");
        }

        let voter_rep = Self::get_user_reputation(env.clone(), voter.clone()).reputation;
        let vote_weight = if voter_rep > 0 { voter_rep } else { 1 };

        if upvote {
            post.upvotes = post
                .upvotes
                .checked_add(vote_weight)
                .expect("upvote overflow");
        } else {
            post.downvotes = post
                .downvotes
                .checked_add(vote_weight)
                .expect("downvote overflow");
        }

        env.storage().persistent().set(&vote_key, &upvote);
        env.storage()
            .persistent()
            .set(&DataKey::Post(post_id), &post);

        // Update author reputation
        let reputation_change = if upvote { vote_weight } else { -vote_weight };
        Self::update_user_reputation(&env, &post.author, |rep| UserReputation {
            reputation: rep.reputation + reputation_change,
            upvotes_received: if upvote {
                rep.upvotes_received + vote_weight
            } else {
                rep.upvotes_received
            },
            downvotes_received: if !upvote {
                rep.downvotes_received + vote_weight
            } else {
                rep.downvotes_received
            },
            ..rep
        });

        env.events().publish(
            (symbol_short!("FRM"), symbol_short!("POST_VOTE"), post_id),
            (voter, upvote, vote_weight),
        );
    }

    pub fn vote_reply(env: Env, voter: Address, reply_id: u32, upvote: bool) {
        voter.require_auth();

        let mut reply = Self::get_reply(env.clone(), reply_id);

        let vote_key = DataKey::ReplyVote(reply_id, voter.clone());
        if env.storage().persistent().has(&vote_key) {
            panic!("already voted");
        }

        let voter_rep = Self::get_user_reputation(env.clone(), voter.clone()).reputation;
        let vote_weight = if voter_rep > 0 { voter_rep } else { 1 };

        if upvote {
            reply.upvotes = reply
                .upvotes
                .checked_add(vote_weight)
                .expect("upvote overflow");
        } else {
            reply.downvotes = reply
                .downvotes
                .checked_add(vote_weight)
                .expect("downvote overflow");
        }

        env.storage().persistent().set(&vote_key, &upvote);
        env.storage()
            .persistent()
            .set(&DataKey::Reply(reply_id), &reply);

        // Update author reputation
        let reputation_change = if upvote { vote_weight } else { -vote_weight };
        Self::update_user_reputation(&env, &reply.author, |rep| UserReputation {
            reputation: rep.reputation + reputation_change,
            upvotes_received: if upvote {
                rep.upvotes_received + vote_weight
            } else {
                rep.upvotes_received
            },
            downvotes_received: if !upvote {
                rep.downvotes_received + vote_weight
            } else {
                rep.downvotes_received
            },
            ..rep
        });

        env.events().publish(
            (symbol_short!("FRM"), symbol_short!("REPLY_V"), reply_id),
            (voter, upvote, vote_weight),
        );
    }

    pub fn mark_best_answer(env: Env, post_author: Address, reply_id: u32) {
        post_author.require_auth();

        let reply = Self::get_reply(env.clone(), reply_id);
        let post = Self::get_post(env.clone(), reply.post_id);

        if post.author != post_author {
            panic!("only post author can mark best answer");
        }

        if post.status != PostStatus::Active {
            panic!("post not active");
        }

        // Remove best answer from any previous reply
        for i in 1..=reply.id {
            if let Some(mut existing_reply) = env
                .storage()
                .persistent()
                .get::<_, Reply>(&DataKey::Reply(i))
            {
                if existing_reply.post_id == reply.post_id && existing_reply.is_best_answer {
                    existing_reply.is_best_answer = false;
                    env.storage()
                        .persistent()
                        .set(&DataKey::Reply(i), &existing_reply);
                }
            }
        }

        let reply_author = reply.author.clone();
        let mut updated_reply = reply;
        updated_reply.is_best_answer = true;
        env.storage()
            .persistent()
            .set(&DataKey::Reply(reply_id), &updated_reply);

        // Reward reply author with bonus reputation
        Self::update_user_reputation(&env, &reply_author, |rep| UserReputation {
            reputation: rep.reputation + 50,
            ..rep
        });

        env.events().publish(
            (symbol_short!("FRM"), symbol_short!("BEST_ANS"), reply_id),
            (post_author, reply_author),
        );
    }

    pub fn moderate_content(
        env: Env,
        moderator: Address,
        target_id: u32,
        is_reply: bool,
        action: ModerationAction,
        _reason: Bytes,
    ) {
        // Prevent reentrancy via the stake refund transfer on Delete below.
        let _guard = ReentrancyGuard::enter(&env, symbol_short!("FORUM"));

        moderator.require_auth();

        let mod_threshold: u32 = env
            .storage()
            .persistent()
            .get(&MODERATION_THRESHOLD)
            .unwrap_or(DEFAULT_MODERATION_THRESHOLD);
        let moderator_rep = Self::get_user_reputation(env.clone(), moderator.clone()).reputation;
        if moderator_rep < mod_threshold as i128 {
            panic!("insufficient reputation for moderation");
        }

        if is_reply {
            let mut reply = Self::get_reply(env.clone(), target_id);
            reply.moderation_count = reply
                .moderation_count
                .checked_add(1)
                .expect("moderation count overflow");

            match action {
                ModerationAction::Hide => {
                    let post = Self::get_post(env.clone(), reply.post_id);
                    if post.status == PostStatus::Active {
                        let mut updated_post = post;
                        updated_post.status = PostStatus::Hidden;
                        env.storage()
                            .persistent()
                            .set(&DataKey::Post(reply.post_id), &updated_post);
                    }
                }
                ModerationAction::Delete => {
                    // Refund stakes proportionally
                    let refund_amount = reply.stake_amount / 2;
                    Self::transfer_tokens(
                        &env,
                        &env.current_contract_address(),
                        &reply.author,
                        refund_amount,
                    );
                }
                _ => {}
            }

            env.storage()
                .persistent()
                .set(&DataKey::Reply(target_id), &reply);
        } else {
            let mut post = Self::get_post(env.clone(), target_id);
            post.moderation_count = post
                .moderation_count
                .checked_add(1)
                .expect("moderation count overflow");

            match action {
                ModerationAction::Hide => post.status = PostStatus::Hidden,
                ModerationAction::Lock => post.status = PostStatus::Locked,
                ModerationAction::Delete => {
                    let was_active = post.status == PostStatus::Active;
                    post.status = PostStatus::Deleted;
                    if was_active {
                        let active_posts = env.storage().persistent().get(&DataKey::ActivePosts).unwrap_or(0u32);
                        env.storage().persistent().set(&DataKey::ActivePosts, &active_posts.saturating_sub(1));
                    }
                    // Refund stakes proportionally
                    let refund_amount = post.stake_amount / 2;
                    Self::transfer_tokens(
                        &env,
                        &env.current_contract_address(),
                        &post.author,
                        refund_amount,
                    );
                }
            }

            env.storage()
                .persistent()
                .set(&DataKey::Post(target_id), &post);
        }

        // Record moderation action
        env.storage()
            .persistent()
            .set(&DataKey::Moderation(target_id, moderator.clone()), &action);

        env.events().publish(
            (symbol_short!("FRM"), symbol_short!("MOD_ACT"), target_id),
            (moderator, is_reply, action),
        );
    }

    pub fn create_category(env: Env, admin: Address, name: Bytes, description: Bytes) -> u32 {
        let admin_address: Address = env
            .storage()
            .persistent()
            .get(&ADMIN)
            .expect("not initialized");
        if admin != admin_address {
            panic!("only admin can create categories");
        }
        admin.require_auth();

        let mut count: u32 = env.storage().persistent().get(&CATEGORY_COUNT).unwrap_or(0);
        count = count.checked_add(1).expect("category count overflow");

        let now = env.ledger().timestamp();

        let category = Category {
            id: count,
            name,
            description,
            created_at: now,
            post_count: 0,
            is_active: true,
        };

        env.storage().persistent().set(&CATEGORY_COUNT, &count);
        env.storage()
            .persistent()
            .set(&DataKey::Category(count), &category);

        env.events().publish(
            (symbol_short!("FRM"), symbol_short!("CAT_CREAT"), count),
            admin,
        );

        count
    }

    // View functions
    pub fn get_post(env: Env, id: u32) -> Post {
        env.storage()
            .persistent()
            .get(&DataKey::Post(id))
            .expect("post not found")
    }

    pub fn get_reply(env: Env, id: u32) -> Reply {
        env.storage()
            .persistent()
            .get(&DataKey::Reply(id))
            .expect("reply not found")
    }

    pub fn get_category(env: Env, id: u32) -> Category {
        env.storage()
            .persistent()
            .get(&DataKey::Category(id))
            .expect("category not found")
    }

    pub fn get_user_reputation(env: Env, address: Address) -> UserReputation {
        env.storage()
            .persistent()
            .get(&DataKey::UserReputation(address.clone()))
            .unwrap_or(UserReputation {
                address: address.clone(),
                reputation: 0,
                posts_created: 0,
                replies_created: 0,
                upvotes_received: 0,
                downvotes_received: 0,
                moderation_actions: 0,
            })
    }

    pub fn get_forum_stats(env: Env) -> ForumStats {
        ForumStats {
            total_posts: env.storage().persistent().get(&DataKey::TotalPosts).unwrap_or(0),
            total_replies: env.storage().persistent().get(&DataKey::TotalReplies).unwrap_or(0),
            active_posts: env.storage().persistent().get(&DataKey::ActivePosts).unwrap_or(0),
            total_staked_mnt: env.storage().persistent().get(&DataKey::TotalStakedMnt).unwrap_or(0),
        }
    }

    pub fn get_post_count(env: Env) -> u32 {
        env.storage().persistent().get(&POST_COUNT).unwrap_or(0)
    }

    pub fn get_posts_page(env: Env, offset: u32, limit: u32) -> Vec<u32> {
        let count: u32 = env.storage().persistent().get(&POST_COUNT).unwrap_or(0);
        let (start, end) = Pagination::bounds(count, offset, limit);
        let mut posts = Vec::new(&env);
        // Post IDs are sequential and 1-based.
        for i in start..end {
            posts.push_back(i + 1);
        }
        posts
    }

    pub fn get_posts_by_category(
        env: Env,
        category_id: u32,
        offset: u32,
        limit: u32,
    ) -> Vec<u32> {
        let category = Self::get_category(env.clone(), category_id);
        let (start, end) = Pagination::bounds(category.post_count, offset, limit);
        let mut posts = Vec::new(&env);
        for i in start..end {
            if let Some(post_id) = env
                .storage()
                .persistent()
                .get::<_, u32>(&DataKey::CategoryIndex(category_id, i))
            {
                posts.push_back(post_id);
            }
        }
        posts
    }

    pub fn get_replies_page(env: Env, post_id: u32, offset: u32, limit: u32) -> Vec<u32> {
        let post = Self::get_post(env.clone(), post_id);
        let (start, end) = Pagination::bounds(post.reply_count, offset, limit);
        let mut replies = Vec::new(&env);
        // Reply IDs are sequential and 1-based within a post.
        for i in start..end {
            let reply_id = i + 1;
            if let Some(reply) = env
                .storage()
                .persistent()
                .get::<_, Reply>(&DataKey::Reply(reply_id))
            {
                if reply.post_id == post_id {
                    replies.push_back(reply_id);
                }
            }
        }
        replies
    }

    pub fn get_replies_by_post(env: Env, post_id: u32, limit: u32) -> Vec<u32> {
        let mut replies = Vec::new(&env);
        let max_results = limit;
        let mut found = 0u32;

        for current_id in 1..=1000 {
            // Reasonable limit to prevent infinite loops
            if found >= max_results {
                break;
            }
            if let Some(reply) = env
                .storage()
                .persistent()
                .get::<_, Reply>(&DataKey::Reply(current_id))
            {
                if reply.post_id == post_id {
                    replies.push_back(current_id);
                    found += 1;
                }
            }
        }

        replies
    }

    // Helper functions
    fn create_default_categories(env: &Env) {
        let _admin: Address = env.storage().persistent().get(&ADMIN).unwrap();

        let categories = vec![
            env,
            (
                Bytes::from_slice(env, b"General Discussion"),
                Bytes::from_slice(env, b"General topics and discussions"),
            ),
            (
                Bytes::from_slice(env, b"Technical Support"),
                Bytes::from_slice(env, b"Get help with technical issues"),
            ),
            (
                Bytes::from_slice(env, b"Feature Requests"),
                Bytes::from_slice(env, b"Suggest new features and improvements"),
            ),
            (
                Bytes::from_slice(env, b"Bug Reports"),
                Bytes::from_slice(env, b"Report bugs and issues"),
            ),
            (
                Bytes::from_slice(env, b"Announcements"),
                Bytes::from_slice(env, b"Official announcements and news"),
            ),
        ];

        for (name, description) in categories {
            let mut count: u32 = env.storage().persistent().get(&CATEGORY_COUNT).unwrap_or(0);
            count = count.checked_add(1).expect("category count overflow");

            let category = Category {
                id: count,
                name,
                description,
                created_at: env.ledger().timestamp(),
                post_count: 0,
                is_active: true,
            };

            env.storage().persistent().set(&CATEGORY_COUNT, &count);
            env.storage()
                .persistent()
                .set(&DataKey::Category(count), &category);
        }
    }

    fn update_user_reputation(
        env: &Env,
        address: &Address,
        updater: impl FnOnce(UserReputation) -> UserReputation,
    ) {
        let current = Self::get_user_reputation(env.clone(), address.clone());
        let updated = updater(current);
        env.storage()
            .persistent()
            .set(&DataKey::UserReputation(address.clone()), &updated);
    }

    fn update_category_post_count(env: &Env, category_id: u32, delta: u32) {
        let mut category = Self::get_category(env.clone(), category_id);
        category.post_count = category
            .post_count
            .checked_add(delta)
            .expect("category post count overflow");
        env.storage()
            .persistent()
            .set(&DataKey::Category(category_id), &category);
    }

    fn transfer_tokens(env: &Env, from: &Address, to: &Address, amount: i128) {
        let token = Self::token_address(env);
        let fn_name = Symbol::new(env, "transfer");
        let args = vec![
            env,
            from.clone().into_val(env),
            to.clone().into_val(env),
            amount.into_val(env),
        ];
        env.invoke_contract::<()>(&token, &fn_name, args);
    }

    fn token_address(env: &Env) -> Address {
        env.storage()
            .persistent()
            .get(&TOKEN)
            .expect("token not set")
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use shared::MAX_PAGE_SIZE;
    use soroban_sdk::testutils::{Address as _, Ledger};

    #[contract]
    pub struct MockMntToken;

    #[contractimpl]
    impl MockMntToken {
        pub fn balance(env: Env, addr: Address) -> i128 {
            env.storage()
                .persistent()
                .get(&(symbol_short!("BAL"), addr))
                .unwrap_or(0)
        }

        pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
            from.require_auth();

            let from_balance = Self::balance(env.clone(), from.clone());
            if from_balance < amount {
                panic!("insufficient balance");
            }

            env.storage()
                .persistent()
                .set(&(symbol_short!("BAL"), from), &(from_balance - amount));

            let to_balance = Self::balance(env.clone(), to.clone());
            env.storage()
                .persistent()
                .set(&(symbol_short!("BAL"), to), &(to_balance + amount));
        }

        pub fn mint(env: Env, to: Address, amount: i128) {
            let balance = Self::balance(env.clone(), to.clone());
            env.storage()
                .persistent()
                .set(&(symbol_short!("BAL"), to), &(balance + amount));
        }
    }

    #[test]
    fn test_forum_initialization() {
        let env = Env::default();
        env.mock_all_auths();

        let forum_id = env.register_contract(None, ForumContract);
        let token_id = env.register_contract(None, MockMntToken);
        let forum = ForumContractClient::new(&env, &forum_id);

        let admin = Address::generate(&env);
        forum.initialize(
            &admin,
            &token_id,
            &Some(10i128),
            &Some(1_000_000i128),
            &Some(3u32),
        );

        // Verify default categories were created
        let category1 = forum.get_category(&1);
        assert_eq!(
            category1.name,
            Bytes::from_slice(&env, b"General Discussion")
        );
    }

    #[test]
    fn test_create_post() {
        let env = Env::default();
        env.mock_all_auths();

        let forum_id = env.register_contract(None, ForumContract);
        let token_id = env.register_contract(None, MockMntToken);
        let forum = ForumContractClient::new(&env, &forum_id);
        let token = MockMntTokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let author = Address::generate(&env);

        forum.initialize(
            &admin,
            &token_id,
            &Some(10i128),
            &Some(1_000_000i128),
            &Some(3u32),
        );
        token.mint(&author, &10_000_000i128);

        let title = Bytes::from_slice(&env, b"Test Question");
        let content_hash = BytesN::from_array(&env, &[1u8; 32]);

        let post_id = forum.create_post(
            &author,
            &title,
            &content_hash,
            &PostType::Question,
            &1, // General Discussion
            &1_000_000i128,
        );

        let post = forum.get_post(&post_id);
        assert_eq!(post.author, author);
        assert_eq!(post.post_type, PostType::Question);
        assert_eq!(post.stake_amount, 1_000_000i128);
    }

    #[test]
    fn test_voting_system() {
        let env = Env::default();
        env.mock_all_auths();

        let forum_id = env.register_contract(None, ForumContract);
        let token_id = env.register_contract(None, MockMntToken);
        let forum = ForumContractClient::new(&env, &forum_id);
        let token = MockMntTokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let author = Address::generate(&env);
        let voter = Address::generate(&env);

        forum.initialize(
            &admin,
            &token_id,
            &Some(10i128),
            &Some(1_000_000i128),
            &Some(3u32),
        );
        token.mint(&author, &10_000_000i128);
        token.mint(&voter, &10_000_000i128);

        // Create post
        let title = Bytes::from_slice(&env, b"Test Question");
        let content_hash = BytesN::from_array(&env, &[1u8; 32]);
        let post_id = forum.create_post(
            &author,
            &title,
            &content_hash,
            &PostType::Question,
            &1,
            &1_000_000i128,
        );

        // Vote on post
        forum.vote_post(&voter, &post_id, &true);

        let post = forum.get_post(&post_id);
        assert_eq!(post.upvotes, 1); // Default reputation is 1

        let reputation = forum.get_user_reputation(&author);
        assert_eq!(reputation.reputation, 1);
    }

    #[test]
    fn test_reply_and_best_answer() {
        let env = Env::default();
        env.mock_all_auths();

        let forum_id = env.register_contract(None, ForumContract);
        let token_id = env.register_contract(None, MockMntToken);
        let forum = ForumContractClient::new(&env, &forum_id);
        let token = MockMntTokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let author = Address::generate(&env);
        let responder = Address::generate(&env);

        forum.initialize(
            &admin,
            &token_id,
            &Some(10i128),
            &Some(1_000_000i128),
            &Some(3u32),
        );
        token.mint(&author, &10_000_000i128);
        token.mint(&responder, &10_000_000i128);

        // Create post
        let title = Bytes::from_slice(&env, b"How to learn Rust?");
        let content_hash = BytesN::from_array(&env, &[1u8; 32]);
        let post_id = forum.create_post(
            &author,
            &title,
            &content_hash,
            &PostType::Question,
            &1,
            &1_000_000i128,
        );

        // Create reply
        let reply_hash = BytesN::from_array(&env, &[2u8; 32]);
        let reply_id = forum.create_reply(&responder, &post_id, &reply_hash, &1_000_000i128);

        // Mark as best answer
        forum.mark_best_answer(&author, &reply_id);

        let reply = forum.get_reply(&reply_id);
        assert!(reply.is_best_answer);

        let responder_rep = forum.get_user_reputation(&responder);
        assert_eq!(responder_rep.reputation, 50); // Best answer bonus
    }

    /// A token whose `transfer` re-enters the forum contract, simulating a
    /// malicious token attempting to exploit the stake transfer flow.
    #[contract]
    pub struct ReentrantToken;

    #[contractimpl]
    impl ReentrantToken {
        pub fn set_forum(env: Env, forum: Address) {
            env.storage()
                .persistent()
                .set(&symbol_short!("FORUM"), &forum);
        }

        pub fn set_reenter(env: Env, on: bool) {
            env.storage()
                .persistent()
                .set(&symbol_short!("REENTER"), &on);
        }

        pub fn transfer(env: Env, _from: Address, _to: Address, _amount: i128) {
            let reenter: bool = env
                .storage()
                .persistent()
                .get(&symbol_short!("REENTER"))
                .unwrap_or(false);
            if reenter {
                let forum_addr: Address = env
                    .storage()
                    .persistent()
                    .get(&symbol_short!("FORUM"))
                    .unwrap();
                let forum = ForumContractClient::new(&env, &forum_addr);
                let title = Bytes::from_slice(&env, b"reentrant");
                let content_hash = BytesN::from_array(&env, &[7u8; 32]);
                forum.create_post(
                    &env.current_contract_address(),
                    &title,
                    &content_hash,
                    &PostType::Question,
                    &1,
                    &1_000_000i128,
                );
            }
        }
    }

    #[test]
    fn test_reentrancy_guard_fires_on_create_post() {
        let env = Env::default();
        env.mock_all_auths();

        let forum_id = env.register_contract(None, ForumContract);
        let token_id = env.register_contract(None, ReentrantToken);
        let forum = ForumContractClient::new(&env, &forum_id);
        let token = ReentrantTokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let author = Address::generate(&env);

        forum.initialize(
            &admin,
            &token_id,
            &Some(10i128),
            &Some(1_000_000i128),
            &Some(3u32),
        );
        token.set_forum(&forum_id);
        token.set_reenter(&true);

        let title = Bytes::from_slice(&env, b"Attack");
        let content_hash = BytesN::from_array(&env, &[1u8; 32]);

        // The transfer re-enters create_post, which must be rejected by the guard.
        let result = forum.try_create_post(
            &author,
            &title,
            &content_hash,
            &PostType::Question,
            &1,
            &1_000_000i128,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_reentrancy_guard_fires_on_create_reply() {
        let env = Env::default();
        env.mock_all_auths();

        let forum_id = env.register_contract(None, ForumContract);
        let token_id = env.register_contract(None, ReentrantToken);
        let forum = ForumContractClient::new(&env, &forum_id);
        let token = ReentrantTokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let author = Address::generate(&env);
        let responder = Address::generate(&env);

        forum.initialize(
            &admin,
            &token_id,
            &Some(10i128),
            &Some(1_000_000i128),
            &Some(3u32),
        );
        token.set_forum(&forum_id);

        // Create the post with reentrancy disabled.
        token.set_reenter(&false);
        let title = Bytes::from_slice(&env, b"Question");
        let content_hash = BytesN::from_array(&env, &[1u8; 32]);
        let post_id = forum.create_post(
            &author,
            &title,
            &content_hash,
            &PostType::Question,
            &1,
            &1_000_000i128,
        );

        // Enable reentrancy and attempt the reply.
        token.set_reenter(&true);
        let reply_hash = BytesN::from_array(&env, &[2u8; 32]);
        let result = forum.try_create_reply(&responder, &post_id, &reply_hash, &1_000_000i128);

        assert!(result.is_err());
    }

    #[test]
    fn test_reentrancy_guard_fires_on_moderate_delete() {
        let env = Env::default();
        env.mock_all_auths();

        let forum_id = env.register_contract(None, ForumContract);
        let token_id = env.register_contract(None, ReentrantToken);
        let forum = ForumContractClient::new(&env, &forum_id);
        let token = ReentrantTokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let author = Address::generate(&env);
        let voter = Address::generate(&env);

        // Threshold 1 so a single upvote is enough to moderate.
        forum.initialize(
            &admin,
            &token_id,
            &Some(10i128),
            &Some(1_000_000i128),
            &Some(1u32),
        );
        token.set_forum(&forum_id);

        // Create the post with reentrancy disabled.
        token.set_reenter(&false);
        let title = Bytes::from_slice(&env, b"Post");
        let content_hash = BytesN::from_array(&env, &[1u8; 32]);
        let post_id = forum.create_post(
            &author,
            &title,
            &content_hash,
            &PostType::Question,
            &1,
            &1_000_000i128,
        );

        // Give the author reputation so it can moderate.
        forum.vote_post(&voter, &post_id, &true);
        assert_eq!(forum.get_user_reputation(&author).reputation, 1);

        // Enable reentrancy and moderate-delete, which triggers the stake refund.
        token.set_reenter(&true);
        let reason = Bytes::from_slice(&env, b"spam");
        let result = forum.try_moderate_content(
            &author,
            &post_id,
            &false,
            &ModerationAction::Delete,
            &reason,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_get_posts_page_and_category() {
        let env = Env::default();
        env.mock_all_auths();

        let forum_id = env.register_contract(None, ForumContract);
        let token_id = env.register_contract(None, MockMntToken);
        let forum = ForumContractClient::new(&env, &forum_id);
        let token = MockMntTokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let author = Address::generate(&env);

        forum.initialize(
            &admin,
            &token_id,
            &Some(10i128),
            &Some(1_000_000i128),
            &Some(3u32),
        );
        token.mint(&author, &100_000_000i128);

        let title = Bytes::from_slice(&env, b"Post");
        let content_hash = BytesN::from_array(&env, &[1u8; 32]);

        // Create more posts than MAX_PAGE_SIZE, all in category 1.
        let total = MAX_PAGE_SIZE + 5;
        for _ in 0..total {
            forum.create_post(
                &author,
                &title,
                &content_hash,
                &PostType::Question,
                &1,
                &1_000_000i128,
            );
        }

        assert_eq!(forum.get_post_count(), total);

        // get_posts_page clamps the limit and slices correctly.
        let page = forum.get_posts_page(&0, &u32::MAX);
        assert_eq!(page.len(), MAX_PAGE_SIZE);
        assert_eq!(page.get(0).unwrap(), 1);
        assert_eq!(page.get(MAX_PAGE_SIZE - 1).unwrap(), MAX_PAGE_SIZE);

        let page2 = forum.get_posts_page(&MAX_PAGE_SIZE, &u32::MAX);
        assert_eq!(page2.len(), 5);
        assert_eq!(page2.get(0).unwrap(), MAX_PAGE_SIZE + 1);

        // Out-of-range offset yields an empty page.
        assert_eq!(forum.get_posts_page(&1_000, &10).len(), 0);

        // get_posts_by_category reads the CategoryIndex.
        let cat_page = forum.get_posts_by_category(&1, &0, &u32::MAX);
        assert_eq!(cat_page.len(), MAX_PAGE_SIZE);
        assert_eq!(cat_page.get(0).unwrap(), 1);

        let cat_page2 = forum.get_posts_by_category(&1, &MAX_PAGE_SIZE, &u32::MAX);
        assert_eq!(cat_page2.len(), 5);
        assert_eq!(cat_page2.get(0).unwrap(), MAX_PAGE_SIZE + 1);

        // A category with no posts yields an empty page.
        assert_eq!(forum.get_posts_by_category(&2, &0, &10).len(), 0);
    }

    #[test]
    fn test_get_replies_page() {
        let env = Env::default();
        env.mock_all_auths();

        let forum_id = env.register_contract(None, ForumContract);
        let token_id = env.register_contract(None, MockMntToken);
        let forum = ForumContractClient::new(&env, &forum_id);
        let token = MockMntTokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let author = Address::generate(&env);
        let responder = Address::generate(&env);

        forum.initialize(
            &admin,
            &token_id,
            &Some(10i128),
            &Some(1_000_000i128),
            &Some(3u32),
        );
        token.mint(&author, &100_000_000i128);
        token.mint(&responder, &100_000_000i128);

        let title = Bytes::from_slice(&env, b"Post");
        let content_hash = BytesN::from_array(&env, &[1u8; 32]);
        let post_id = forum.create_post(
            &author,
            &title,
            &content_hash,
            &PostType::Question,
            &1,
            &1_000_000i128,
        );

        let reply_hash = BytesN::from_array(&env, &[2u8; 32]);
        let total = MAX_PAGE_SIZE + 3;
        for _ in 0..total {
            forum.create_reply(&responder, &post_id, &reply_hash, &1_000_000i128);
        }

        // First page is clamped to MAX_PAGE_SIZE.
        let page = forum.get_replies_page(&post_id, &0, &u32::MAX);
        assert_eq!(page.len(), MAX_PAGE_SIZE);
        assert_eq!(page.get(0).unwrap(), 1);
        assert_eq!(page.get(MAX_PAGE_SIZE - 1).unwrap(), MAX_PAGE_SIZE);

        // Second page holds the remainder.
        let page2 = forum.get_replies_page(&post_id, &MAX_PAGE_SIZE, &u32::MAX);
        assert_eq!(page2.len(), 3);
        assert_eq!(page2.get(0).unwrap(), MAX_PAGE_SIZE + 1);

        // Out-of-range offset yields an empty page.
        assert_eq!(forum.get_replies_page(&post_id, &1_000, &10).len(), 0);
    }
}
