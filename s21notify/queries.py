# -*- coding: utf-8 -*-
"""Тексты GraphQL-операций платформы.

ВНИМАНИЕ: GraphQL-шлюз platform.21-school.ru принимает только запросы из белого
списка — текст должен ДОСЛОВНО совпадать с тем, что шлёт официальный веб-клиент
(иначе HTTP 400 c 'REQUEST_ABSENT_IN_WHITELISTS' в заголовке x-bad-request).
Не менять форматирование и набор полей.

Тексты операций взяты из https://github.com/s21toolkit/s21schema (MIT).
"""

# ---- предстоящие проверки (я проверяю / меня проверяют) ---------------------
BOOKINGS_OP = "calendarGetMyReviews"
BOOKINGS_QUERY = """fragment ProjectTeamMember on User {
  id
  avatarUrl
  login
  userExperience {
    level {
      id
      range {
        levelCode
      }
    }
    cookiesCount
    codeReviewPoints
  }
  activeSchoolShortName
}

fragment ProjectTeamMembers on ProjectTeamMembers {
  id
  teamLead {
    ...ProjectTeamMember
  }
  members {
    ...ProjectTeamMember
  }
  invitedUsers {
    ...ProjectTeamMember
  }
  teamName
  teamStatus
  minTeamMemberCount
  maxTeamMemberCount
}

fragment UserInBooking on User {
  id
  login
  avatarUrl
  userExperience {
    level {
      id
      range {
        levelCode
      }
    }
  }
}

fragment Review on CalendarBooking {
  id
  answerId
  eventSlot {
    id
    start
    end
  }
  task {
    id
    title
    assignmentType
    goalId
    goalName
    studentTaskAdditionalAttributes {
      cookiesCount
    }
  }
  verifierUser {
    ...UserInBooking
  }
  verifiableStudent {
    id
    user {
      ...UserInBooking
    }
  }
  team {
    ...ProjectTeamMembers
  }
  bookingStatus
  isOnline
  vcLinkUrl
}

query calendarGetMyReviews($to: DateTime, $limit: Int) {
  student {
    getMyUpcomingBookings(to: $to, limit: $limit) {
      ...Review
    }
  }
}"""

# ---- лента уведомлений платформы («колокольчик») ----------------------------
NOTIFICATIONS_OP = "getUserNotifications"
NOTIFICATIONS_QUERY = """query getUserNotifications($paging: PagingInput!) {
  s21Notification {
    getS21Notifications(paging: $paging) {
      notifications {
        id
        relatedObjectType
        relatedObjectId
        message
        time
        wasRead
        groupName
      }
      totalCount
      groupNames
    }
  }
}"""

# ---- дедлайны ---------------------------------------------------------------
DEADLINES_OP = "deadlinesGetDeadlines"
DEADLINES_QUERY = """fragment Level on ExperienceLevelRange {
  id
  level
  levelCode
  leftBorder
  rightBorder
}

fragment GoalCourse on CourseCoverInformation {
  localCourseId
  courseName
  courseType
  experienceFact
  finalPercentage
  displayedCourseStatus
}

fragment DeadlineGoalData on DeadlineGoal {
  goalProjects {
    studentGoalId
    project {
      goalName
      goalId
    }
    status
    executionType
    finalPercentage
    finalPoint
    pointTask
  }
  goalCourses {
    ...GoalCourse
  }
  levels {
    ...Level
  }
}

fragment DeadlineData on Deadline {
  deadlineId
  description
  comment
  deadlineToDaysArray
  deadlineTs
  createTs
  updateTs
  status
  rules {
    logicalOperatorId
    rulesInGroup {
      logicalOperatorId
      value {
        fieldId
        subFieldKey
        operator
        value
      }
    }
  }
}

query deadlinesGetDeadlines($deadlineStatuses: [DeadlineStatus!]!, $page: PagingInput!, $deadlinesFrom: DateTime, $deadlinesTo: DateTime, $sorting: [SortingField]) {
  student {
    getDeadlines(
      deadlineStatuses: $deadlineStatuses
      page: $page
      deadlineFrom: $deadlinesFrom
      deadlineTo: $deadlinesTo
      sorting: $sorting
    ) {
      deadline {
        ...DeadlineData
      }
      shiftRequests {
        deadlineShiftRequestId
        status
        daysToShift
        createTs
      }
      deadlineGoal {
        ...DeadlineGoalData
      }
      shiftCount
    }
  }
}"""

# ---- экзамены ----------------------------------------------------------------
EXAMS_OP = "calendarGetExams"
EXAMS_QUERY = """fragment CalendarExam on Exam {
  examId
  eventId
  beginDate
  endDate
  name
  location
  maxStudentCount
  currentStudentsCount
  updateDate
  goalId
  goalName
  isWaitListActive
  isInWaitList
  stopRegisterDate
}

query calendarGetExams($from: DateTime!, $to: DateTime!) {
  student {
    getExams(from: $from, to: $to) {
      ...CalendarExam
    }
  }
}"""

# ---- агенда (мои события календаря) -------------------------------------------
AGENDA_OP = "getAgendaEvents"
AGENDA_QUERY = """query getAgendaEvents($from: DateTime!, $to: DateTime!, $limit: Int!) {
  calendarEventS21 {
    getMyAgendaEvents(from: $from, to: $to, limit: $limit) {
      agendaItemContext {
        entityId
        entityType
      }
      start
      end
      label
      description
      agendaEventType
      additionalInfo {
        key
        value
      }
    }
  }
}"""

# ---- опыт/уровень/коины --------------------------------------------------------
EXPERIENCE_OP = "getCurrentUserExperience"
EXPERIENCE_QUERY = """fragment CurrentUserExperience on UserExperience {
  id
  cookiesCount
  codeReviewPoints
  coinsCount
  level {
    id
    range {
      id
      levelCode
    }
  }
}

query getCurrentUserExperience {
  student {
    getExperience {
      ...CurrentUserExperience
    }
  }
}"""

# ---- активные P2P-заявки --------------------------------------------------------
P2P_OP = "calendarGetMyActualP2pRequests"
P2P_QUERY = """fragment CalendarP2pRequest on P2pRequest {
  p2pRequestId
  startTime
  endTime
  isOnline
  goalId
  goalName
  studentAnswerId
}

query calendarGetMyActualP2pRequests($from: DateTime!, $to: DateTime!) {
  student {
    getMyActualP2pRequests(from: $from, to: $to) {
      ...CalendarP2pRequest
    }
  }
}"""
